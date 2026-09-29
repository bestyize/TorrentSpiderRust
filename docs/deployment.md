# 部署

服务端是单个静态链接的可执行文件（网页已编译进去），进程交给 systemd 管理。
发布包里的 `torrent-spider.sh` 负责安装、升级、回滚和日常运维。

要求：x86_64 Linux、systemd、root 权限。

## 1. 拿到发布包

在 GitHub 仓库的 Actions 页运行「Build Linux server」，产物在该次运行页面底部的 Artifacts 里。
也可以用命令行：

```sh
gh workflow run build-linux-server.yml -f target=x86_64-unknown-linux-musl
gh run download --name torrent-spider-server-0.1.0-x86_64-unknown-linux-musl
sha256sum -c torrent-spider-server-*.tar.gz.sha256
```

默认的 musl 版本不依赖系统库，任何 x86_64 发行版都能跑。

## 2. 安装

```sh
tar -xzf torrent-spider-server-0.1.0-x86_64-unknown-linux-musl.tar.gz
cd torrent-spider-server-0.1.0-x86_64-unknown-linux-musl
sudo ./torrent-spider.sh install
```

安装会启动服务、设为开机自启，并等到 `/api/health` 能响应、违禁过滤就绪后才返回。装好的文件：

| 路径 | 用途 |
|---|---|
| `/opt/torrent-spider/torrent-spider-server` | 程序；升级后旧版本留在同目录的 `.prev` |
| `/etc/torrent-spider/torrent-spider.env` | 配置，升级不覆盖 |
| `/etc/systemd/system/torrent-spider.service` | systemd 单元 |
| `/var/cache/torrent-spider/` | 大陆 IP 段的落盘缓存 |
| `/usr/local/bin/torrent-spider` | 本脚本，之后在任意目录直接用 |

服务以 systemd 临时用户（`DynamicUser`）在沙箱里运行，除了缓存目录，整个文件系统对它只读。

## 3. 配置

编辑 `/etc/torrent-spider/torrent-spider.env`，改完执行 `sudo torrent-spider restart`。

| 变量 | 默认 | 说明 |
|---|---|---|
| `HOST` | `0.0.0.0` | 监听地址。只给本机反向代理用时改成 `127.0.0.1` |
| `PORT` | `55555` | 监听端口 |
| `TORRENT_PROXY` | 无 | 出站代理，`host:port` 或 `socks5://host:port`。站点和大陆 IP 段都走它 |
| `TRUST_FORWARDED_FOR` | `0` | 部署在反向代理后面时必须设 `1`，见第 5 节 |
| `SENSITIVE_FILTER` | `1` | 违禁内容过滤总开关 |
| `SENSITIVE_IP_LIST_URL` | 17mon/china_ip_list | 大陆 IP 段来源，GitHub 不稳时可换镜像 |
| `RUST_LOG` | `info,tower_http=info` | 日志级别 |

服务没有鉴权，默认监听所有网卡。

## 4. 日常运维

```sh
torrent-spider status                     # 状态、版本、内存、违禁过滤是否就绪
torrent-spider logs                       # 跟踪日志；也可传 journalctl 参数，如 logs --since today
sudo torrent-spider restart               # 改完配置后重启
sudo torrent-spider stop | start
```

停止时服务收到 SIGTERM，会等进行中的搜索处理完再退出，最多 60 秒。

**升级**：把新发布包传到服务器上，执行

```sh
sudo torrent-spider upgrade torrent-spider-server-<新版本>-x86_64-unknown-linux-musl.tar.gz
```

脚本先试跑新二进制，确认能在这台机器上执行；然后备份旧版本、替换、重启，等健康检查通过。
新版本起不来时自动退回旧版本，命令返回失败并打印日志。

**回滚**：`sudo torrent-spider rollback` 退回上一个版本；再执行一次回到当前版本。

**卸载**：`sudo torrent-spider uninstall` 保留配置；加 `--purge` 连配置和缓存一起删。

## 5. 反向代理与 `TRUST_FORWARDED_FOR`

违禁过滤要判断请求方是不是中国大陆 IP，默认看 TCP 对端地址，客户端伪造不了。
放在 nginx / Caddy 后面时，对端变成了反向代理自己，所有请求都会被判成「不是大陆」，过滤静默失效。
这时要在配置里设：

```sh
HOST=127.0.0.1
TRUST_FORWARDED_FOR=1
```

服务改用 `X-Forwarded-For` 的最后一段（反向代理追加的那一段）判断来源。
直连部署不要开：开了之后任何人加一行 `X-Forwarded-For: 8.8.8.8` 就能绕过过滤。

nginx：

```nginx
location / {
    proxy_pass http://127.0.0.1:55555;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
}
```

配好后从外网验证，`client_ip` 应该是访客的公网 IP，而不是 `127.0.0.1`：

```sh
curl -s https://你的域名/api/health
```

## 6. 防火墙

走反向代理时只对外开 80/443，不要开 55555：

```sh
sudo ufw allow 80/tcp
sudo ufw allow 443/tcp
```

## 7. 排查

- **违禁过滤未就绪**（`status` 里显示「未就绪」，或 `/api/health` 的 `ready` 为 `false`）：大陆 IP 段没拉到，
  这时不会过滤任何人。多半是服务器访问不了 `raw.githubusercontent.com`，配置 `TORRENT_PROXY` 或
  `SENSITIVE_IP_LIST_URL` 后重启。拉到过一次之后会落盘缓存，之后断网也能用。
- **同一个 WiFi 下测试看不到过滤效果**：服务端只看得到局域网地址，判不出大陆。
  用 `/api/health` 的 `client_ip` 确认服务端看到的来源。
- **启动失败**：脚本会打印最近的日志；完整日志用 `torrent-spider logs -n 200 --no-pager`。
  最常见的是端口被占用，改 `PORT` 后重启。

## 不用脚本手动部署

把 `torrent-spider-server` 放到 `/opt/torrent-spider/`，`torrent-spider.env` 放到 `/etc/torrent-spider/`，
`torrent-spider.service` 放到 `/etc/systemd/system/`，然后：

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now torrent-spider
```
