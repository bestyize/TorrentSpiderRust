/*
 * TorrentSpider 磁力站前端。
 *
 * 零构建：原生 ES 模块、无打包器、无框架。所有外部数据都用 createElement +
 * textContent 落地，绝不拼 innerHTML——站点标题里出现 <em> 之类的高亮标签是常态，
 * 一旦拼字符串就会把上游 HTML 注进页面。
 */
(() => {
    'use strict';

    const form = document.getElementById('search-form');
    const queryInput = document.getElementById('query');
    const sourceSelect = document.getElementById('source');
    const optimizeToggle = document.getElementById('optimize');
    const submitButton = document.getElementById('submit');
    const progress = document.getElementById('progress');
    const statusRow = document.getElementById('status');
    const results = document.getElementById('results');
    const toast = document.getElementById('toast');
    const modal = document.getElementById('modal');
    const modalTitle = document.getElementById('modal-title');
    const modalSubtitle = document.getElementById('modal-subtitle');
    const modalBody = document.getElementById('modal-body');
    const modalClose = document.getElementById('modal-close');

    /** src 编号 -> 站点标题。下拉框和结果卡都要用。 */
    const sourceTitles = new Map();

    /** 当前已展示结果对应的查询，翻页时复用；null 表示还没搜过。 */
    let active = null; // { keyword, src, filter, optimize, page }
    /** 已出现过的条目键，翻页去重用——和 Compose 端 itemKey 的口径一致。 */
    let seenKeys = new Set();
    let loading = false;
    /** 最近一次整页失败的详情；「知道了」只收起显示，不清这个值。 */
    let lastError = null;

    /** 打开弹窗的按钮，关闭后把焦点还给它。焦点不还原的话键盘用户会被丢回页首。 */
    let modalOpener = null;
    /** 弹窗里正在进行的磁力请求的后端标识。关闭后到达的响应据此丢弃。 */
    let modalMagnetController = null;

    // ---------- 工具 ----------

    const el = (tag, className, text) => {
        const node = document.createElement(tag);
        if (className) node.className = className;
        if (text != null) node.textContent = text;
        return node;
    };

    /**
     * 把库用空串表示「没有」的字段当作空值。
     * 服务端已经做了 "" -> null 的归一，这里再兜一层：
     * 以后哪个站点解析器漏了归一，前端也不会渲染出一个空 chip。
     */
    const value = (raw) => (typeof raw === 'string' && raw.trim() !== '' ? raw : null);

    /** 与核心库 itemKey 一致：详情页地址 > 标题 > 哈希 > 磁力。 */
    const itemKey = (item) =>
        value(item.detail_url) || value(item.title) || value(item.hash) || value(item.magnet_url) || '';

    const sourceTitle = (code) => sourceTitles.get(code) || `来源 ${code}`;

    let toastTimer = 0;

    /**
     * 弹提示。
     *
     * 这里刻意**不用** requestAnimationFrame 触发入场动画：rAF 在后台标签页、
     * 无头浏览器、以及部分省电模式下会被节流甚至完全停摆，一旦回调不跑，
     * `is-visible` 就永远加不上——元素停在 opacity:0，同时隐藏定时器也没注册，
     * 结果就是「点了没反应」。改成先让元素进去、强制一次 reflow，
     * 再挂 `is-visible`，这样入场动画照旧，但绝不依赖某一帧回调是否被执行。
     */
    function showToast(message, tone = 'info') {
        toast.textContent = message;
        // 两个 tone class 都要显式开关：只 toggle 其中一个的话，
        // 上一次的成功态不会自动消失，下一次失败会顶着绿色显示
        toast.classList.toggle('is-success', tone === 'success');
        toast.classList.toggle('is-error', tone === 'error');
        toast.hidden = false;
        toast.classList.remove('is-visible');
        // 强制同步 reflow，让上面的 remove 立刻生效，否则紧接着的 add
        // 可能被合并成同一次样式计算，过渡不会触发
        void toast.offsetWidth;
        toast.classList.add('is-visible');

        clearTimeout(toastTimer);
        toastTimer = setTimeout(() => {
            toast.classList.remove('is-visible');
            // 等过渡结束再真正移除，避免动画被截断
            setTimeout(() => {
                toast.hidden = true;
            }, 260);
        }, 2400);
    }

    /**
     * 给 promise 加超时。
     *
     * navigator.clipboard.writeText 在权限被拒、页面失焦、或浏览器把它挂在
     * 一个永不 settle 的 promise 上时，既不 resolve 也不 reject。原实现是
     * `await` 它之后再 showToast，于是超时那条路根本走不到——用户点下去
     * 什么都不会发生。这里超时即当失败处理。
     */
    function withTimeout(promise, ms) {
        return new Promise((resolve, reject) => {
            const timer = setTimeout(() => reject(new Error('clipboard 超时')), ms);
            promise.then(
                (value) => {
                    clearTimeout(timer);
                    resolve(value);
                },
                (error) => {
                    clearTimeout(timer);
                    reject(error);
                },
            );
        });
    }

    /** 旧式兜底：select + execCommand。需要仍在用户手势的调用栈里才有效。 */
    function legacyCopy(text) {
        const scratch = document.createElement('textarea');
        scratch.value = text;
        scratch.setAttribute('readonly', '');
        scratch.style.position = 'fixed';
        scratch.style.top = '0';
        scratch.style.opacity = '0';
        document.body.appendChild(scratch);
        scratch.select();
        scratch.setSelectionRange(0, text.length);
        let ok = false;
        try {
            ok = document.execCommand('copy');
        } catch {
            ok = false;
        }
        document.body.removeChild(scratch);
        return ok;
    }

    /**
     * 两条复制路径都失败时的兜底：把磁力原文摊在卡片里供用户手动选中。
     *
     * 之前只说了一句「请手动选择」，却没给任何可选的文本——用户根本无从下手。
     * 这里渲染一个 readonly 的 textarea：内容可全选，`select()` 也已调用，
     * 用户按 Ctrl/Cmd+C 即可。
     */
    function showManualCopy(card, text) {
        const existing = card.querySelector('.manual-copy');
        if (existing) existing.remove();

        const box = el('div', 'manual-copy');
        const area = document.createElement('textarea');
        area.className = 'manual-copy-area';
        area.value = text;
        area.readOnly = true;
        area.rows = 2;
        area.setAttribute('aria-label', '磁力链接，可手动复制');
        box.appendChild(area);
        box.appendChild(el(
            'p',
            'manual-copy-hint',
            '请按 Ctrl/Cmd + C 复制。两条自动复制路径都没走通，' +
            '多半是浏览器的剪贴板权限限制（非 https 的局域网地址会这样）。',
        ));
        card.appendChild(box);

        // 顺序要紧：先同步把内容设好、再聚焦。
        // 若先 focus 再赋值，部分浏览器会在聚焦触发的可访问性树更新里
        // 抢先播报一次「编辑框，空白」，读屏用户会以为框是空的。
        area.focus();
        area.select();
    }

    /**
     * 复制并给出**就地**反馈。
     *
     * [button] 可选：成功时按钮文案临时变成「已复制」，失败时变成「请手动复制」。
     * 只靠底部 toast 的坑在于它离按钮很远、又只停 2.4 秒——用户点完眼睛还停在
     * 按钮上，很容易整场都没注意到。就地改按钮文案是零延迟的确认。
     */
    async function copyText(text, button, card) {
        const restore = button ? button.textContent : null;
        const flash = (label, className) => {
            if (!button) return;
            button.textContent = label;
            button.classList.add(className);
            setTimeout(() => {
                button.textContent = restore;
                button.classList.remove(className);
            }, 1600);
        };

        // 优先 Clipboard API；它需要安全上下文（https 或 localhost），
        // 局域网 IP 明文直连时取不到。超时 600ms：再久用户已经觉得没反应了。
        if (navigator.clipboard && navigator.clipboard.writeText) {
            try {
                await withTimeout(navigator.clipboard.writeText(text), 600);
                showToast('已复制磁力链接', 'success');
                flash('已复制', 'is-copied');
                return;
            } catch {
                // 落到 execCommand
            }
        }

        if (legacyCopy(text)) {
            showToast('已复制磁力链接', 'success');
            flash('已复制', 'is-copied');
            return;
        }

        // 两条路都失败：把磁力摊在卡片里，用户还能手动选中。
        // 文案不写「下方」——弹窗里那条文本框在按钮上方，写死方向会自相矛盾。
        showToast('复制失败，请手动选择磁力链接', 'error');
        flash('请手动复制', 'is-copied-fail');
        if (card) showManualCopy(card, text);
    }

    // ---------- 磁力弹窗 ----------

    /**
     * 打开弹窗。
     *
     * 弹窗外壳（背景遮罩、面板、关闭按钮）在 index.html 里，这里只换三处文字：
     * 标题、副标题（资源标题，用来确认点对了哪条）、正文。正文按状态重建。
     *
     * 遮罩层不加 body overflow:hidden：遮罩已经盖住整页、并吃掉了滚轮事件，
     * 而改 body 的滚动样式会让页面在关闭时跳一下，代价比收益大。
     */
    /**
     * 把焦点送进弹窗正文，文本框则顺带全选。
     *
     * 必须在内容已经进 DOM **之后**调用：还在 DocumentFragment 里时
     * focus() 是空操作，select() 也落不下去。
     *
     * 磁力就绪时全选而不是只聚焦——用户点开弹窗十有八九就是要复制，
     * 整条选中后只剩一次 Ctrl/Cmd+C。这也兑现了正文里那句「已经全选」。
     */
    function focusModalBody() {
        const target = modalBody.querySelector('button, textarea, [tabindex]');
        if (!target) return;
        target.focus();
        if (target.tagName === 'TEXTAREA') target.select();
    }

    function openModal({ title, subtitle, body, opener }) {
        modalOpener = opener || null;
        modalTitle.textContent = title;
        modalSubtitle.textContent = subtitle || '';
        modalSubtitle.hidden = !subtitle;
        modalBody.replaceChildren(body);
        modal.hidden = false;
        // 焦点移进弹窗：不这么做的话 Tab 会继续在背后的搜索结果里走，
        // 用户用键盘根本关不掉这个弹窗
        focusModalBody();
    }

    function closeModal() {
        if (modal.hidden) return;
        // 关掉时中止在途请求。不中止的话，一个已经没人等的详情页抓取
        // 还会继续占着上游连接。
        if (modalMagnetController) {
            modalMagnetController.abort();
            modalMagnetController = null;
        }
        modal.hidden = true;
        modalBody.replaceChildren();
        if (modalOpener && document.contains(modalOpener)) {
            modalOpener.focus();
        }
        modalOpener = null;
    }

    /** 磁力就绪后的正文：文本框 + 复制按钮 + 种子地址捷径。 */
    function magnetReadyBody(magnet, torrentUrl) {
        const fragment = document.createDocumentFragment();

        const area = document.createElement('textarea');
        area.className = 'manual-copy-area';
        area.value = magnet;
        area.readOnly = true;
        area.rows = 3;
        area.setAttribute('aria-label', '磁力链接');
        fragment.appendChild(area);

        const actions = el('div', 'modal-actions');
        actions.style.marginTop = '12px';

        const copyButton = el('button', 'button button-primary', '复制磁力');
        copyButton.type = 'button';
        // 传 card=null：弹窗里的失败兜底不能往卡片里塞，文本框本来就在眼前
        copyButton.addEventListener('click', () => copyText(magnet, copyButton, null));
        actions.appendChild(copyButton);

        if (torrentUrl) {
            const torrentLink = el('a', 'button', '下载种子文件');
            torrentLink.href = torrentUrl;
            // itorrents 常拿 301 换域名；带 noopener 是为了别把 window.opener
            // 交给第三方站点
            torrentLink.rel = 'noopener';
            torrentLink.target = '_blank';
            actions.appendChild(torrentLink);
        }

        fragment.appendChild(actions);
        fragment.appendChild(el('p', 'modal-hint', '点「复制磁力」即可粘贴到下载工具；已经全选，也可以直接 Ctrl/Cmd + C。'));

        // 聚焦与全选交给 openModal 在插入 DOM 之后做（内容还没进 DOM 时
        // focus/select 都是空操作）
        return fragment;
    }

    function magnetLoadingBody() {
        const box = el('div', 'modal-loading');
        box.appendChild(el('span', 'spinner'));
        box.appendChild(el('span', null, '正在获取磁力链接…'));
        return box;
    }

    /** 主动取磁力失败时的正文。“来源没有磁力”和“请求没成”是两回事。 */
    function magnetErrorBody(message, retry) {
        const box = el('div', 'modal-error');
        box.appendChild(el('p', null, message));
        const actions = el('div', 'modal-actions');
        actions.style.marginTop = '14px';
        const retryButton = el('button', 'button button-primary', '重试');
        retryButton.type = 'button';
        retryButton.addEventListener('click', retry);
        actions.appendChild(retryButton);
        box.appendChild(actions);
        return box;
    }

    /**
     * 取磁力。
     *
     * 两个入口共用：结果里已经带了磁力（不发请求，直接展示），
     * 以及没有磁力（打 /api/magnet 让服务端去抓详情页）。
     *
     * 只有「没有磁力」这一条路才发请求——用户要的是「有就直接看」，
     * 为了统一形状而多发一次请求会让「直接」变成「等一下」。
     */
    function resolveMagnet({ src, detailUrl, knownMagnet, subtitle, opener, silent }) {
        if (knownMagnet) {
            // 已经有磁力：不进加载态，不发生任何请求。
            // torrentUrl 传 null：结果里带的磁力没有让服务端推导过，
            // 这里不自己拼（拼错会指向不存在的种子），只给磁力本身。
            openModal({
                title: '磁力链接',
                subtitle,
                body: magnetReadyBody(knownMagnet, null),
                opener,
            });
            return;
        }

        // silent：失败后点「重试」时不再重新打开弹窗（弹窗已经开着）
        if (!silent) {
            openModal({
                title: '正在获取磁力',
                subtitle,
                body: magnetLoadingBody(),
                opener,
            });
        } else {
            modalBody.replaceChildren(magnetLoadingBody());
        }

        const run = async () => {
            const controller = new AbortController();
            modalMagnetController = controller;
            const params = new URLSearchParams({ src: String(src), detail_url: detailUrl });

            let payload = null;
            try {
                const response = await fetch(`/api/magnet?${params}`, { signal: controller.signal });
                payload = await response.json().catch(() => null);
                if (!response.ok) {
                    // 服务端把「站点没提供磁力」翻译成 404 + MAGNET_UNAVAILABLE，
                    // 它的 message 已经能直接给用户看；其余错误才退回通用文案
                    const fallback = `获取失败（HTTP ${response.status}）`;
                    throw new Error((payload && payload.message) || fallback);
                }
            } catch (error) {
                // 关闭弹窗触发的 abort 不是错误，静默退出即可
                if (error && error.name === 'AbortError') return;
                const message = error && error.message ? error.message : '获取磁力链接失败。';
                showMagnetError(message, { src, detailUrl, subtitle, opener });
                return;
            } finally {
                if (modalMagnetController === controller) modalMagnetController = null;
            }

            // 请求在弹窗关闭后才回来：此时弹窗已重置，不能再往上写
            if (modal.hidden) return;

            const magnet = payload && payload.magnet_url ? payload.magnet_url : null;
            if (!magnet) {
                showMagnetError('服务端没有返回磁力链接。', { src, detailUrl, subtitle, opener });
                return;
            }

            modalTitle.textContent = '磁力链接';
            modalBody.replaceChildren(magnetReadyBody(magnet, payload.torrent_url));
            // 与 openModal 里同样的收尾：内容换掉后要重新把焦点/全选落到新文本框上
            focusModalBody();
        };

        // run() 内部已把所有失败路径翻译成弹窗内容；再挂一层兜底，
        // 免得将来漏掉的异常变成静默的 unhandled rejection（控制台一片红却没人知道哪来的）
        run().catch(() => {});
    }

    /** 取磁力失败：换掉弹窗正文，并给一个「重试」。 */
    function showMagnetError(message, { src, detailUrl, subtitle, opener }) {
        modalTitle.textContent = '没能获取磁力';
        modalBody.replaceChildren(magnetErrorBody(message, () => resolveMagnet({
            src, detailUrl, subtitle, opener, silent: true,
        })));
        const first = modalBody.querySelector('button');
        if (first) first.focus();
    }

    /** 结果卡上的「获取磁力」入口。 */
    function openMagnetModal(item, index, opener) {
        const detailUrl = value(item.detail_url);
        const knownMagnet = value(item.magnet_url);
        const src = item.src;
        const subtitle = value(item.title) || `第 ${index + 1} 条结果`;

        // 既没磁力又没详情页：无从下手。按钮本该禁用，这里兜一层，
        // 免得以后解析器改动后出现点了没反应的按钮。
        if (!knownMagnet && !detailUrl) {
            openModal({
                title: '无法获取磁力',
                subtitle,
                body: (() => {
                    const box = el('div', 'modal-error');
                    box.appendChild(el('p', null, `来源「${sourceTitle(src)}」的这条结果里既没有磁力链接，也没有详情页地址，无法进一步解析。`));
                    return box;
                })(),
                opener,
            });
            return;
        }

        // knownMagnet 优先于 detailUrl：结果里已经有磁力就不该再去抓详情页。
        resolveMagnet({ src, detailUrl, knownMagnet, subtitle, opener });
    }

    modalClose.addEventListener('click', closeModal);
    // 点背景关闭。挂在背景这一层而不是外层的 .modal 上，
    // 否则点面板内部也会冒泡到 .modal 从而误关。
    modal.querySelector('[data-close]').addEventListener('click', closeModal);
    document.addEventListener('keydown', (event) => {
        if (event.key === 'Escape' && !modal.hidden) {
            event.preventDefault();
            closeModal();
        }
    });
    // 兜底：万一有别的路径把 hidden 去掉了，离开时也要清掉在途请求
    window.addEventListener('pagehide', closeModal);

    // ---------- 加载态 ----------

    function setLoading(isLoading) {
        loading = isLoading;
        form.classList.toggle('is-loading', isLoading);
        submitButton.disabled = isLoading;
        progress.classList.toggle('is-active', isLoading);
        if (isLoading) {
            progress.classList.remove('is-done');
        }
    }

    function finishLoading() {
        progress.classList.remove('is-active');
        progress.classList.add('is-done');
        // 让进度条走满再淡出，重置回 0 时不能有过渡，否则会看到它倒着缩回去
        setTimeout(() => {
            progress.style.transition = 'none';
            progress.classList.remove('is-done');
            progress.style.removeProperty('transition');
        }, 600);
    }

    function renderSkeletons(count = 4) {
        results.replaceChildren();
        for (let i = 0; i < count; i += 1) {
            results.appendChild(el('div', 'skeleton'));
        }
    }

    /**
     * 空态/错误态的占位块。
     *
     * [action] 是可选的收尾动作（错误态用「知道了」）。对齐 Compose 端
     * App.kt 的错误处理：那边是 AlertDialog + 「知道了」，这里换成内联块，
     * 但同样要有一个「我看到了」的出口——否则错误信息会一直占着结果区，
     * 用户不知道下一步该做什么。
     */
    function renderEmpty(title, subtitle, action) {
        results.replaceChildren();
        const box = el('div', 'empty');
        box.appendChild(el('div', 'empty-dot'));
        box.appendChild(el('h2', null, title));
        box.appendChild(el('p', null, subtitle));
        if (action) {
            const button = el('button', 'button', action.label);
            button.type = 'button';
            button.addEventListener('click', action.onClick);
            box.appendChild(button);
        }
        results.appendChild(box);
    }

    /**
     * 渲染错误态。收起后不放空结果区——那样看起来像「搜到了 0 条」，
     * 而实际是「压根没搜成」。改为留一条可点回来的窄条。
     */
    function showError() {
        if (!lastError) return;
        renderEmpty('搜索失败', lastError.message, {
            label: '知道了',
            onClick: () => {
                results.replaceChildren();
                const bar = el('button', 'error-bar');
                bar.type = 'button';
                // 用 textContent 拼，不碰 innerHTML：message 里带着上游 URL
                // 和站点返回的片段，属于外部输入
                bar.textContent = `搜索失败：${lastError.message}（点此查看）`;
                bar.addEventListener('click', showError);
                results.appendChild(bar);
            },
        });
    }

    // ---------- 结果卡 ----------

    function buildCard(item, index) {
        const card = el('article', 'card');

        const head = el('div', 'card-head');
        head.appendChild(el('h2', 'card-title', value(item.title) || '未命名条目'));
        head.appendChild(el('span', 'card-index', `#${index + 1}`));
        card.appendChild(head);

        // chip 顺序固定：[来源][大小][日期][下载 N][做种 N]
        const chips = el('div', 'chips');
        chips.appendChild(el('span', 'chip chip-source', sourceTitle(item.src)));

        const size = value(item.size);
        if (size) chips.appendChild(el('span', 'chip chip-size', size));

        const date = value(item.date);
        if (date) chips.appendChild(el('span', 'chip chip-date', date));

        const downloads = value(item.download_count);
        if (downloads) chips.appendChild(el('span', 'chip chip-download', `下载 ${downloads}`));

        const seeds = value(item.sender_count);
        if (seeds) chips.appendChild(el('span', 'chip chip-seed', `做种 ${seeds}`));

        card.appendChild(chips);

        // 只留「获取磁力」一个动作。详情页链接与哈希复制都已移除：
        // 详情页 302/反爬后的落地页经常对不上标题，点过去多半是失望；
        // 哈希本身是磁力链接的子串，复制磁力已经覆盖了这个用途。
        //
        // 为什么是「获取」而不是「复制」：TorLock、1337 这类站的搜索结果里
        // 压根没有磁力，磁力只挂在详情页上。按钮改成打开弹窗，有就直接展示、
        // 没有就让服务端现抓一次，两种情况的落点是同一个弹窗。
        const actions = el('div', 'card-actions');

        const detailUrl = value(item.detail_url);
        const magnet = value(item.magnet_url);
        const magnetButton = el('button', 'button button-primary', '获取磁力');
        magnetButton.type = 'button';
        if (magnet || detailUrl) {
            magnetButton.addEventListener('click', () => openMagnetModal(item, index, magnetButton));
        } else {
            // 既没磁力也没详情页：给不出磁力，按钮保留但禁用并说明原因，
            // 比直接不显示更好懂
            magnetButton.disabled = true;
            magnetButton.title = '这条结果里既没有磁力链接，也没有详情页地址';
        }
        actions.appendChild(magnetButton);

        card.appendChild(actions);
        return card;
    }

    function renderItems(items, append) {
        if (!append) {
            results.replaceChildren();
        }
        // 去掉「加载更多」那一块，稍后按需重建
        const existingMore = results.querySelector('.loadmore');
        if (existingMore) existingMore.remove();

        const startIndex = append ? results.querySelectorAll('.card').length : 0;
        let added = 0;
        items.forEach((item) => {
            const key = itemKey(item);
            if (seenKeys.has(key)) return;
            seenKeys.add(key);
            results.appendChild(buildCard(item, startIndex + added));
            added += 1;
        });
        return added;
    }

    function renderLoadMore() {
        const box = el('div', 'loadmore');
        box.appendChild(el('span', 'loadmore-note', `第 ${active.page + 1} 页`));
        const button = el('button', 'button button-primary', '加载更多');
        button.type = 'button';
        button.addEventListener('click', () => runSearch({ page: active.page + 1, append: true }));
        box.appendChild(button);
        results.appendChild(box);
    }

    function setStatus(text) {
        statusRow.textContent = text || '';
        statusRow.hidden = !text;
    }

    /**
     * 汇总每个源的状态。
     *
     * 核心库把「站点改版」和「真的没结果」都收敛成空列表，外界分不出来；
     * 至少能把「抓取直接失败」的源明确说出来，用户才知道结果为什么偏少。
     */
    function buildStatusText(payload, added) {
        const failures = (payload.sources || []).filter((s) => !s.ok);
        const parts = [`“${payload.keyword}” 返回 ${added} 条结果`];

        // 过滤掉多少要主动说出来。服务端给的 sources[].count 是「站点返回了多少条」
        // （在过滤之前统计的），和 items 的长度一比就是被丢掉的数量。
        // 不显示这个数的话，同一个关键词开关一拨结果就少一大截，用户无从判断
        // 是「过滤生效了」还是「这次站点抽风了」。
        const rawCount = (payload.sources || [])
            .reduce((sum, s) => sum + (s.ok ? s.count : 0), 0);
        const shown = (payload.items || []).length;
        const hidden = Math.max(0, rawCount - shown);
        if (hidden > 0) {
            parts.push(`已隐藏 ${hidden} 条与关键词无关的`);
        }

        // 搜索优化真的多打了几轮时才说。不说的话结果比预期多，用户会以为站点抽风；
        // 而 queries 为空（没有可折的数字，或用户关掉了优化）时这话就不该出现。
        const variants = payload.queries || [];
        if (variants.length > 0) {
            parts.push(`同时按「${variants.join('」「')}」各搜了一遍`);
        }

        if (payload.src == null) {
            const scanned = (payload.sources || []).length;
            parts.push(`已扫描 ${scanned} 个来源`);
            if (failures.length > 0) {
                parts.push(`失败 ${failures.length} 个：${failures.map((s) => s.title).join('、')}`);
            }
        } else {
            parts.push(`来源：${sourceTitle(payload.src)}`);
        }
        return parts.join('，') + '。';
    }

    // ---------- 请求 ----------

    async function runSearch({ page = 1, append = false } = {}) {
        if (loading) return;

        const keyword = queryInput.value.trim();
        const src = sourceSelect.value;
        // 一个开关驱动服务端的两个参数：补搜其它数字写法（optimize）与隐藏无关结果（filter）。
        // 两者都让「搜出来的更贴近你要找的」，合成一个开关；关掉就都关掉，
        // 回到「只按字面搜一次、原样列出上游给的全部结果」。
        const optimized = optimizeToggle.checked;
        const filter = optimized;
        const optimize = optimized;
        if (!keyword) {
            queryInput.focus();
            return;
        }

        // 换关键字、换来源或拨开关时都是「新搜索」，必须从第 1 页重来：
        // 否则会出现第 2 页的结果接在另一个关键字的结果后面。
        // filter / optimize 也必须参与判断——口径变了，旧的那几页跟新的对不上。
        const isNewSearch = !active
            || active.keyword !== keyword
            || active.src !== src
            || active.filter !== filter
            || active.optimize !== optimize;
        if (isNewSearch && page !== 1) page = 1;
        const effectiveAppend = append && !isNewSearch;

        setLoading(true);
        if (!effectiveAppend) {
            active = { keyword, src, filter, optimize, page: 1 };
            seenKeys = new Set();
            setStatus('正在搜索…');
            renderSkeletons();
        } else {
            setStatus(`正在加载第 ${page} 页…`);
        }

        const params = new URLSearchParams({
            q: keyword,
            page: String(page),
            src,
            filter: filter ? '1' : '0',
            optimize: optimize ? '1' : '0',
        });
        try {
            const response = await fetch(`/api/search?${params}`);
            const payload = await response.json().catch(() => null);

            if (!response.ok) {
                const message = (payload && payload.message) || `请求失败（HTTP ${response.status}）`;
                throw new Error(message);
            }

            const added = renderItems(payload.items || [], effectiveAppend);
            active.page = payload.page;
            // 这一次请求成功了，上一次的错误已经不再是当前状态；
            // 不清掉的话「知道了」会出现在一份正常结果下面，语义就乱了
            lastError = null;

            if (effectiveAppend) {
                setStatus(`第 ${payload.page} 页已加载，新增 ${added} 条结果。`);
            } else {
                setStatus(buildStatusText(payload, added));
            }

            if ((payload.items || []).length === 0 && !effectiveAppend) {
                renderEmpty('还没有结果', '换个关键字，或者把搜索源切到「全部来源」再试一次。');
            } else if ((payload.items || []).length > 0) {
                // 能返回满一页就假定还有下一页；站点没有总数接口，
                // 与其猜一个假的总页数，不如让用户点了才知道
                renderLoadMore();
            } else {
                const more = results.querySelector('.loadmore');
                if (more) more.remove();
            }
        } catch (error) {
            const message = error && error.message ? error.message : '搜索失败';
            setStatus('搜索失败');
            if (!effectiveAppend) {
                // 把失败信息存下来，这样「知道了」收起后还能再展开；
                // 点一下就永久消失的话，用户想回头确认错误原因就没了
                lastError = { message };
                showError();
            } else {
                // 分页失败时整页内容还在，用 toast 而不是占满结果区
                showToast(message, 'error');
            }
        } finally {
            setLoading(false);
            finishLoading();
        }
    }

    async function loadSources() {
        try {
            const response = await fetch('/api/sources');
            if (!response.ok) return;
            const payload = await response.json();
            const sources = (payload.sources || []).slice()
                .sort((a, b) => String(a.title).localeCompare(String(b.title), 'zh-Hans-CN'));

            sources.forEach((source) => {
                sourceTitles.set(source.src, source.title);
                const option = document.createElement('option');
                option.value = String(source.src);
                option.textContent = source.title;
                sourceSelect.appendChild(option);
            });
        } catch {
            // 源列表拉不到不影响搜索：src=all 仍然可用，只是后端少了下拉项。
            // 这里刻意不弹错误，避免首次打开就一串红字。
        }
    }

    form.addEventListener('submit', (event) => {
        event.preventDefault();
        runSearch({ page: 1, append: false });
    });

    queryInput.addEventListener('keydown', (event) => {
        if (event.key === 'Enter') {
            event.preventDefault();
            runSearch({ page: 1, append: false });
        }
    });

    // 拨开关立刻重搜：这跟改关键字是同一类操作，都需要重新问一次服务端
    // （补搜发生在服务端，过滤也发生在服务端，前端手上没有能自己筛的原始数据）。
    // 没搜过时不动——首屏还没结果，重搜没有意义。
    optimizeToggle.addEventListener('change', () => {
        if (active) runSearch({ page: 1, append: false });
    });

    renderEmpty('还没有结果', '输入关键字并执行搜索，结果会在这里按卡片展示。');
    loadSources();
})();
