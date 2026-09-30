#!/usr/bin/env bash
#
# TorrentSpider 服务端的安装与运维脚本，进程交给 systemd 管理。
#
# 在解压后的发布包目录里：
#   sudo ./torrent-spider.sh install              安装并启动
#
# 安装后脚本会复制到 /usr/local/bin/torrent-spider，之后在任意目录：
#   sudo torrent-spider status                    运行状态、内存、违禁过滤是否就绪
#   torrent-spider logs [journalctl 参数]          跟踪日志
#   sudo torrent-spider start | stop | restart
#   sudo torrent-spider upgrade <发布包.tar.gz>     升级；新版本起不来会自动退回旧版本
#   sudo torrent-spider rollback                  退回上一个版本
#   sudo torrent-spider uninstall [--purge]       卸载；--purge 连配置和缓存一起删
#
# 配置文件是 /etc/torrent-spider/torrent-spider.env，改完执行 restart 生效。
# 部署说明见发布包里的 DEPLOY.md（仓库里的 docs/deployment.md）。

set -euo pipefail

readonly SERVICE=torrent-spider
readonly BIN_NAME=torrent-spider-server
# 用来确认 /usr/local/bin 里那个文件确实是本脚本，卸载时才敢删
readonly MARKER='TorrentSpider 服务端的安装与运维脚本'

# 安装位置。默认值与 torrent-spider.service 里写的路径一致；改了的话安装单元文件时会一并替换
INSTALL_DIR=${INSTALL_DIR:-/opt/torrent-spider}
CONFIG_DIR=${CONFIG_DIR:-/etc/torrent-spider}
UNIT_DIR=${UNIT_DIR:-/etc/systemd/system}
CACHE_ROOT=${CACHE_ROOT:-/var/cache}
LINK_PATH=${LINK_PATH:-/usr/local/bin/torrent-spider}
# 等服务能响应、等违禁过滤就绪的上限（秒）
START_TIMEOUT=${START_TIMEOUT:-30}
READY_TIMEOUT=${READY_TIMEOUT:-45}

BIN_PATH=$INSTALL_DIR/$BIN_NAME
UNIT_PATH=$UNIT_DIR/$SERVICE.service
UNIT_BACKUP=$INSTALL_DIR/$SERVICE.service.prev
ENV_PATH=$CONFIG_DIR/$SERVICE.env
SCRIPT_PATH=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/$(basename -- "${BASH_SOURCE[0]}")
SCRIPT_DIR=$(dirname -- "$SCRIPT_PATH")

ACTION=
WORK_DIR=
NEW_VERSION=
PKG_BIN='' PKG_UNIT='' PKG_ENV='' PKG_SCRIPT=''

say() { printf '%s\n' "$*"; }
warn() { printf '%s\n' "$*" >&2; }
die() {
  warn "错误：$*"
  exit 1
}

cleanup() {
  if [[ -n $WORK_DIR ]]; then rm -rf -- "$WORK_DIR"; fi
}
trap cleanup EXIT

# ---------- 前置检查 ----------

require_systemd() {
  command -v systemctl >/dev/null 2>&1 || die "需要 systemd（找不到 systemctl）"
}

# 路径最近的已存在祖先目录是否可写。比判断 EUID 更准：没有写权限时后面的 install/rm 一定会失败
writable() {
  local path=$1
  while [[ ! -e $path ]]; do path=$(dirname -- "$path"); done
  [[ -w $path ]]
}

require_root() {
  local path
  for path in "$INSTALL_DIR" "$CONFIG_DIR" "$UNIT_DIR" "$LINK_PATH"; do
    writable "$path" || die "没有写 $path 的权限，请用 sudo 执行：sudo $0 $ACTION"
  done
}

is_installed() { [[ -f $UNIT_PATH ]]; }

require_installed() {
  is_installed || die "还没有安装。在解压后的发布包里执行：sudo ./torrent-spider.sh install"
}

# ---------- 配置与健康检查 ----------

# 取配置文件里某个键最后一次赋值的值，去掉两侧空白和引号；文件不存在或读不到时输出空串
config_value() {
  local key=$1 file=${2:-$ENV_PATH}
  [[ -r $file ]] || return 0
  sed -n -E "s/^[[:space:]]*${key}[[:space:]]*=[[:space:]]*//p" "$file" | tail -n 1 |
    sed -E -e 's/[[:space:]]+$//' -e 's/^"(.*)"$/\1/' -e "s/^'(.*)'$/\1/"
}

service_url() {
  local host port
  host=$(config_value HOST)
  port=$(config_value PORT)
  case $host in
    '' | 0.0.0.0) host=127.0.0.1 ;;
    ::) host='[::1]' ;;
    *:*) host="[$host]" ;;
  esac
  printf 'http://%s:%s' "$host" "${port:-55555}"
}

# 成功时输出响应体；请求失败返回非零，本机没有 curl/wget 时返回 2
http_get() {
  if command -v curl >/dev/null 2>&1; then
    curl -fsS --max-time 3 "$1" 2>/dev/null
  elif command -v wget >/dev/null 2>&1; then
    wget -qO- -T 3 "$1" 2>/dev/null
  else
    return 2
  fi
}

# 从 /api/health 的紧凑 JSON 里取一个标量字段，不依赖 jq
json_field() {
  local match
  match=$(printf '%s' "$1" | grep -o "\"$2\":[^,}]*" | head -n 1) || return 0
  match=${match#*:}
  printf '%s' "${match//\"/}"
}

# 端口是否被占用：0 被占、1 空闲、2 判断不了（没有 ss）
port_in_use() {
  command -v ss >/dev/null 2>&1 || return 2
  ss -ltn "sport = :$1" 2>/dev/null | tail -n +2 | grep -q .
}

# 用户主动执行的启动 / 重启先清掉失败计数：systemd 默认 10 秒内启动超过 5 次就拒绝再启动
# （start-limit-hit），连着改几次配置、或升级失败回滚之后紧接着再操作，都会撞上这条限制
service_start() {
  systemctl reset-failed "$SERVICE" 2>/dev/null || true
  systemctl start "$SERVICE"
}

service_restart() {
  systemctl reset-failed "$SERVICE" 2>/dev/null || true
  systemctl restart "$SERVICE"
}

# 等到 /api/health 能响应。进程起来不等于能用：配置写错、端口被占都会让它起来又立刻退出
wait_healthy() {
  local url i=0 rc
  url=$(service_url)/api/health
  while ((i < START_TIMEOUT * 2)); do
    # Restart=on-failure 会在几秒后重启崩溃的进程，那期间状态不是 active，直接判失败
    systemctl is-active --quiet "$SERVICE" || return 1
    rc=0
    http_get "$url" >/dev/null || rc=$?
    if ((rc == 0)); then return 0; fi
    if ((rc == 2)); then
      warn "提示：本机没有 curl 或 wget，跳过健康检查"
      return 0
    fi
    sleep 0.5
    i=$((i + 1))
  done
  return 1
}

show_failure() {
  warn ""
  warn "最近的日志："
  journalctl -u "$SERVICE" -n 20 --no-pager 2>/dev/null | sed 's/^/    /' >&2 || true
}

wait_healthy_or_die() {
  if ! wait_healthy; then
    show_failure
    die "服务没能正常启动（上面是最近的日志）。修好之后执行：sudo torrent-spider restart"
  fi
}

print_filter() {
  local body=$1 enabled ready lexicon ranges
  if [[ -z $body ]]; then
    say "违禁过滤 : 未知（健康检查没有响应）"
    return
  fi
  enabled=$(json_field "$body" enabled)
  if [[ $enabled != true ]]; then
    say "违禁过滤 : 已关闭"
    return
  fi
  ready=$(json_field "$body" ready)
  lexicon=$(json_field "$body" lexicon_entries)
  ranges=$(json_field "$body" china_ranges)
  if [[ $ready == true ]]; then
    say "违禁过滤 : 已就绪（词表 ${lexicon} 条，大陆 IP 段 ${ranges} 段）"
  else
    say "违禁过滤 : 未就绪（词表 ${lexicon:-0} 条，大陆 IP 段 ${ranges:-0} 段）"
    warn "  大陆 IP 段没拉到，当前不会过滤任何人。多半是这台机器访问不了 raw.githubusercontent.com："
    warn "  在 $ENV_PATH 里配置 TORRENT_PROXY 或 SENSITIVE_IP_LIST_URL 后执行 restart，原因看 torrent-spider logs"
  fi
}

# 违禁词表编在二进制里，大陆 IP 段要启动后联网拉取，几秒到几十秒不等。等它就绪再下结论，
# 否则会对着一个马上就好的服务报「过滤未生效」
wait_ready() {
  local url body enabled ready i=0
  url=$(service_url)/api/health
  while :; do
    body=$(http_get "$url") || body=
    enabled=$(json_field "$body" enabled)
    ready=$(json_field "$body" ready)
    if [[ -z $body || $enabled != true || $ready == true ]] || ((i >= READY_TIMEOUT * 2)); then break; fi
    sleep 0.5
    i=$((i + 1))
  done
  print_filter "$body"
}

# ---------- 发布包与二进制 ----------

# 从参数里找出新二进制、单元文件、配置模板和脚本本身。参数可以是发布包 .tar.gz、
# 解压后的目录或单个二进制；不给时用脚本所在目录（在解压后的发布包里执行的情形）
resolve_package() {
  local arg=${1:-} dir='' found d
  if [[ -z $arg ]]; then
    dir=$SCRIPT_DIR
  elif [[ -d $arg ]]; then
    dir=$(cd -- "$arg" && pwd)
  elif [[ -f $arg && ($arg == *.tar.gz || $arg == *.tgz) ]]; then
    WORK_DIR=$(mktemp -d)
    tar -xzf "$arg" -C "$WORK_DIR" || die "解压失败：$arg"
    found=$(find "$WORK_DIR" -maxdepth 2 -type f -name "$BIN_NAME" -print -quit)
    [[ -n $found ]] || die "发布包里没有 $BIN_NAME：$arg"
    dir=$(dirname -- "$found")
  elif [[ -f $arg ]]; then
    PKG_BIN=$(cd -- "$(dirname -- "$arg")" && pwd)/$(basename -- "$arg")
  else
    die "找不到 $arg"
  fi

  if [[ -n $dir && -f $dir/$BIN_NAME ]]; then PKG_BIN=$dir/$BIN_NAME; fi
  [[ -n $PKG_BIN ]] || die "在 $dir 里找不到 $BIN_NAME。用法：sudo $0 $ACTION <发布包.tar.gz | 二进制路径>"

  # 单元文件、配置模板和脚本优先取发布包里的，没有再用本脚本旁边的
  for d in "$dir" "$SCRIPT_DIR"; do
    [[ -n $d ]] || continue
    if [[ -z $PKG_UNIT && -f $d/$SERVICE.service ]]; then PKG_UNIT=$d/$SERVICE.service; fi
    if [[ -z $PKG_ENV && -f $d/$SERVICE.env ]]; then PKG_ENV=$d/$SERVICE.env; fi
    if [[ -z $PKG_SCRIPT && -f $d/$SERVICE.sh ]]; then PKG_SCRIPT=$d/$SERVICE.sh; fi
  done
  if [[ -z $PKG_SCRIPT ]]; then PKG_SCRIPT=$SCRIPT_PATH; fi
}

# 二进制的版本号。PORT=0：不认 --version 的旧版本会把它当成正常启动，
# 端口 0 让它立刻报错退出，而不是真起一个服务挂在这里
binary_version() {
  local out
  if out=$(PORT=0 HOST=127.0.0.1 "$1" --version 2>/dev/null) && [[ $out == "$BIN_NAME "* ]]; then
    printf '%s' "${out#"$BIN_NAME "}"
  else
    printf '未知版本'
  fi
}

# 先把新二进制放到 .new 并试跑一次 --version：架构不对、文件损坏在这里就会暴露，旧服务不受影响
stage_binary() {
  local version
  mkdir -p -- "$INSTALL_DIR"
  install -m 0755 "$1" "$BIN_PATH.new"
  if ! version=$(PORT=0 HOST=127.0.0.1 "$BIN_PATH.new" --version 2>&1) || [[ $version != "$BIN_NAME "* ]]; then
    rm -f -- "$BIN_PATH.new"
    die "$1 在这台机器（$(uname -m)）上跑不起来，或者不是 $BIN_NAME：${version:-没有输出}"
  fi
  NEW_VERSION=${version#"$BIN_NAME "}
}

# 改名是原子的：正在跑的进程继续用旧文件，重启后才换成新的
activate_binary() { mv -f -- "$BIN_PATH.new" "$BIN_PATH"; }

install_unit() {
  local tmp=$UNIT_PATH.tmp
  mkdir -p -- "$UNIT_DIR"
  sed -e "s|/opt/torrent-spider|$INSTALL_DIR|g" -e "s|/etc/torrent-spider|$CONFIG_DIR|g" "$1" >"$tmp"
  chmod 0644 "$tmp"
  mv -f -- "$tmp" "$UNIT_PATH"
  systemctl daemon-reload
}

# 先写临时文件再改名：覆盖正在执行的脚本本身也不会让它读到半截内容
install_self() {
  local tmp=$LINK_PATH.tmp
  mkdir -p -- "$(dirname -- "$LINK_PATH")"
  install -m 0755 "$PKG_SCRIPT" "$tmp"
  mv -f -- "$tmp" "$LINK_PATH"
}

swap_files() {
  mv -f -- "$1" "$1.swap"
  mv -f -- "$2" "$1"
  mv -f -- "$1.swap" "$2"
}

# 拒绝删除系统目录，防止把安装路径配成 / 之类时 rm -rf 误伤
remove_dir() {
  case $1 in
    '' | / | /bin | /boot | /dev | /etc | /home | /lib | /lib64 | /opt | /proc | /root | /run | /sbin | /srv | /sys | /tmp | /usr | /usr/bin | /usr/local | /usr/local/bin | /usr/sbin | /var | /var/cache | /var/lib)
      die "拒绝删除 $1"
      ;;
  esac
  rm -rf -- "$1"
}

# ---------- 命令 ----------

cmd_install() {
  local port host
  require_systemd
  require_root
  if is_installed; then die "已经安装过了（$UNIT_PATH）。升级用：sudo torrent-spider upgrade <发布包>"; fi
  resolve_package "${1:-}"
  [[ -n $PKG_UNIT ]] || die "找不到 $SERVICE.service，请在解压后的发布包目录里执行"

  # 端口在动手之前查：已有配置沿用，没有就看发布包里的模板
  if [[ -f $ENV_PATH ]]; then port=$(config_value PORT); else port=$(config_value PORT "${PKG_ENV:-/dev/null}"); fi
  port=${port:-55555}
  if port_in_use "$port"; then
    die "端口 $port 已被占用。换个端口：在 ${PKG_ENV:-$ENV_PATH} 里改 PORT 后重新执行 install"
  fi

  stage_binary "$PKG_BIN"
  say "安装 torrent-spider-server $NEW_VERSION"
  mkdir -p -- "$CONFIG_DIR"
  if [[ -f $ENV_PATH ]]; then
    say "  保留已有配置 $ENV_PATH"
  elif [[ -n $PKG_ENV ]]; then
    install -m 0600 "$PKG_ENV" "$ENV_PATH"
  else
    printf 'PORT=%s\n' "$port" >"$ENV_PATH"
    chmod 0600 "$ENV_PATH"
  fi
  activate_binary
  install_unit "$PKG_UNIT"
  install_self
  systemctl enable --quiet "$SERVICE"
  service_start
  wait_healthy_or_die

  say "已启动"
  wait_ready
  host=$(config_value HOST)
  say ""
  say "访问地址 : $(service_url)（其他机器用服务器的 IP）"
  say "配置文件 : $ENV_PATH，改完执行 sudo torrent-spider restart"
  say "常用命令 : torrent-spider status | torrent-spider logs | sudo torrent-spider upgrade <新发布包>"
  if [[ -z $host || $host == 0.0.0.0 || $host == :: ]]; then
    warn ""
    warn "注意：服务监听在所有网卡上，而且没有鉴权。放在反向代理后面时，把 HOST 改成 127.0.0.1"
    warn "      并设置 TRUST_FORWARDED_FOR=1，否则违禁过滤判不出访客来源。详见 DEPLOY.md"
  fi
}

cmd_upgrade() {
  local old_version
  require_systemd
  require_root
  require_installed
  resolve_package "${1:-}"
  old_version=$(binary_version "$BIN_PATH")
  stage_binary "$PKG_BIN"
  say "升级：$old_version → $NEW_VERSION"

  # 旧版本留作回滚：二进制和单元文件各备份一份
  cp -p -- "$BIN_PATH" "$BIN_PATH.prev"
  cp -p -- "$UNIT_PATH" "$UNIT_BACKUP"
  activate_binary
  if [[ -n $PKG_UNIT ]]; then install_unit "$PKG_UNIT"; fi
  install_self
  service_restart
  if wait_healthy; then
    say "升级完成"
    wait_ready
    return 0
  fi

  show_failure
  warn ""
  warn "新版本没能正常启动，退回 $old_version…"
  mv -f -- "$BIN_PATH.prev" "$BIN_PATH"
  mv -f -- "$UNIT_BACKUP" "$UNIT_PATH"
  systemctl daemon-reload
  service_restart
  if wait_healthy; then
    warn "已退回 $old_version，服务恢复正常。新版本的问题看上面的日志"
  else
    show_failure
    warn "退回之后仍然起不来，请检查配置 $ENV_PATH"
  fi
  exit 1
}

cmd_rollback() {
  local current previous
  require_systemd
  require_root
  require_installed
  [[ -f $BIN_PATH.prev ]] || die "没有可以退回的版本（执行过 upgrade 才有）"
  current=$(binary_version "$BIN_PATH")
  previous=$(binary_version "$BIN_PATH.prev")
  say "退回：$current → $previous"
  # 交换而不是覆盖：再执行一次 rollback 就能回到刚才的版本
  swap_files "$BIN_PATH" "$BIN_PATH.prev"
  if [[ -f $UNIT_BACKUP ]]; then
    swap_files "$UNIT_PATH" "$UNIT_BACKUP"
    systemctl daemon-reload
  fi
  service_restart
  wait_healthy_or_die
  say "已退回 $previous；再执行一次 rollback 可以回到 $current"
  wait_ready
}

cmd_start() {
  require_systemd
  require_root
  require_installed
  service_start
  wait_healthy_or_die
  say "已启动"
  wait_ready
}

cmd_restart() {
  require_systemd
  require_root
  require_installed
  service_restart
  wait_healthy_or_die
  say "已重启"
  wait_ready
}

cmd_stop() {
  require_systemd
  require_root
  require_installed
  # systemctl stop 会等进程退出：服务收到 SIGTERM 后等进行中的请求处理完，最多 TimeoutStopSec
  systemctl stop "$SERVICE"
  say "已停止"
}

cmd_status() {
  local state enabled pid since mem body rc=0
  require_systemd
  if ! is_installed; then
    say "未安装"
    return 1
  fi
  state=$(systemctl is-active "$SERVICE" 2>/dev/null) || true
  enabled=$(systemctl is-enabled "$SERVICE" 2>/dev/null) || true
  say "服务     : ${state:-unknown}（开机自启：${enabled:-unknown}）"
  say "版本     : $(binary_version "$BIN_PATH")"
  if [[ -f $BIN_PATH.prev ]]; then say "上一版本 : $(binary_version "$BIN_PATH.prev")（可以 rollback）"; fi
  say "配置文件 : $ENV_PATH"
  [[ $state == active ]] || return 1

  pid=$(systemctl show -p MainPID --value "$SERVICE")
  since=$(systemctl show -p ActiveEnterTimestamp --value "$SERVICE")
  mem=$(systemctl show -p MemoryCurrent --value "$SERVICE")
  say "PID      : $pid"
  say "启动于   : $since"
  if [[ $mem =~ ^[0-9]+$ ]]; then say "内存     : $((mem / 1024 / 1024)) MB"; fi
  # 配置文件是 0600，普通用户读不到端口。不去猜默认端口：同一台机器上那个端口可能是别的程序，
  # 健康检查会问错对象、报出别人的状态
  if [[ ! -r $ENV_PATH ]]; then
    say "健康检查 : 跳过（读不到配置，不知道端口；用 sudo torrent-spider status 查看）"
    return 0
  fi
  say "地址     : $(service_url)"
  body=$(http_get "$(service_url)/api/health") || rc=$?
  case $rc in
    0)
      say "健康检查 : 正常"
      print_filter "$body"
      ;;
    2) say "健康检查 : 跳过（本机没有 curl 或 wget）" ;;
    *)
      warn "健康检查 : 没有响应"
      return 1
      ;;
  esac
}

cmd_logs() {
  require_systemd
  if (($# == 0)); then set -- -n 100 -f; fi
  exec journalctl -u "$SERVICE" "$@"
}

cmd_uninstall() {
  local purge=false
  case ${1:-} in
    --purge) purge=true ;;
    '') ;;
    *) die "未知参数：$1（只支持 --purge）" ;;
  esac
  require_systemd
  require_root
  if is_installed; then
    systemctl disable --now --quiet "$SERVICE" || true
    rm -f -- "$UNIT_PATH"
    systemctl daemon-reload
    systemctl reset-failed "$SERVICE" 2>/dev/null || true
  fi
  remove_dir "$INSTALL_DIR"
  if [[ -f $LINK_PATH ]] && grep -q "$MARKER" "$LINK_PATH"; then rm -f -- "$LINK_PATH"; fi
  if $purge; then
    remove_dir "$CONFIG_DIR"
    # DynamicUser 的缓存实际放在 private/ 下，外面是一个符号链接
    rm -rf -- "${CACHE_ROOT:?}/$SERVICE" "$CACHE_ROOT/private/$SERVICE"
    say "已卸载，配置和缓存也已删除"
  else
    say "已卸载。保留了配置 $CONFIG_DIR；连配置和缓存一起删用 --purge"
  fi
}

usage() {
  cat <<EOF
TorrentSpider 服务端的安装与运维（systemd）

用法：$0 <命令> [参数]

  install [发布包|二进制]   安装并启动；不带参数时用脚本所在目录（解压后的发布包）
  upgrade <发布包|二进制>   升级；新版本起不来会自动退回旧版本
  rollback                 退回上一个版本（再执行一次回到当前版本）
  start | stop | restart   启动、停止、重启
  status                   运行状态、内存、违禁过滤是否就绪
  logs [journalctl 参数]    跟踪日志，默认 -n 100 -f
  uninstall [--purge]      卸载；--purge 连配置和缓存一起删

除 logs 外都需要 root；status 不加 sudo 时只报服务状态，不做健康检查。配置文件：$ENV_PATH
EOF
}

main() {
  ACTION=${1:-help}
  if (($# > 0)); then shift; fi
  case $ACTION in
    install) cmd_install "$@" ;;
    upgrade) cmd_upgrade "$@" ;;
    rollback) cmd_rollback ;;
    start) cmd_start ;;
    stop) cmd_stop ;;
    restart) cmd_restart ;;
    status) cmd_status ;;
    logs | log) cmd_logs "$@" ;;
    uninstall) cmd_uninstall "$@" ;;
    help | -h | --help) usage ;;
    *)
      warn "未知命令：$ACTION"
      usage >&2
      exit 1
      ;;
  esac
}

main "$@"
