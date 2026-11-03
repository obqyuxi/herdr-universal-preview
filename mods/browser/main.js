// browser (herdr-universal-preview): a web page inside Herdr's side panel.
// Type an address and press Enter. Chromium when the host has it (Herdr
// Preview Dev with CEF), else WebKit. Logins are kept in this app's own
// profile, separate from Safari and Chrome.
import { View, div } from "gpui-kit";
import { v_flex, h_flex, InputState, Input } from "gpui-base";
import * as herdr from "herdr";

const ID = "browser";
const HISTORY_MAX = 300;
const hasNative = () => typeof herdr.nativeView?.new === "function";
const engine = () => (herdr.nativeViewCapabilities?.().kinds ?? []).some(k => k.kind === "chromium") ? "chromium" : "web";
const zoomPercent = (level) => Math.round(Math.pow(1.2, level ?? 0) * 100);

export default class Browser extends View {
  init(_props, cx) {
    // No default site: the first time it waits for an address; later it reopens the last page.
    this.start = localStorage.getItem("browser.url") || null;
    this.history = JSON.parse(localStorage.getItem("browser.history") || "[]");
    this.downloads = new Map();
    this.panel = null;          // null | "menu" | "downloads" | "history"
    this.finding = false;
    this.find = null;
    this.zoom = 0;
    this.confirmClear = false;
    this.error = null;
    this.loading = false;
    this.canBack = false;
    this.canForward = false;
    this.title = "";
    this.input = InputState.new({ placeholder: "输入网址，回车打开（例如 github.com 或 localhost:3000）", value: this.start ?? "" });
    this.input.on("submit", (_e, cx) => this.go(this.input.value(), cx));
    this.findInput = InputState.new({ placeholder: "在页面中查找" });
    this.findInput.on("change", (_e, cx) => this.runFind(false, true, cx));
    this.findInput.on("submit", (e, cx) => this.runFind(true, !e?.shift, cx));
    if (!hasNative()) return;
    this.engine = engine();
    cx.spawn(async (cx) => {
      for (;;) {
        const e = JSON.parse(await herdr.nextNativeViewEvent());
        if (e.id !== ID) continue;
        this.onEvent(e.event, e.data ?? {});
        cx.notify();
      }
    });
  }

  onEvent(event, d) {
    if (event === "load.start") { this.loading = true; this.error = null; }
    if (event === "load.commit" || event === "load.finish") {
      this.loading = event !== "load.finish" && this.loading;
      if (event === "load.finish") this.loading = false;
      if (d.url) {
        this.url = d.url;
        if (!this.input.value() || this.panel === null) this.input.set_value(d.url);
        if (!d.url.startsWith("chrome://")) localStorage.setItem("browser.url", d.url);
      }
    }
    if ("title" in d && d.title) this.title = d.title;
    if (event === "load.finish" && d.url && !d.url.startsWith("chrome://")) this.remember(d.url, d.title || this.title);
    if (event === "load.error") { this.loading = false; this.error = d.message || "打开失败"; }
    if (event === "web.processTerminated") this.error = "页面进程意外退出，请点刷新";
    if ("canGoBack" in d) { this.canBack = d.canGoBack; this.canForward = d.canGoForward; }
    if (event === "find.result") this.find = d;
    if (event === "download.start") this.downloads.set(d.id, { id: d.id, name: d.name, percent: 0, complete: false });
    if (event === "import.done") this.importState = { done: d };
    if (event === "import.error") this.importState = { error: d.message || "导入失败" };
    if (event === "download.update") this.downloads.set(d.id, { ...(this.downloads.get(d.id) ?? {}), ...d });
  }

  remember(url, title) {
    if (this.history[0]?.url === url) { this.history[0].title = title || this.history[0].title; }
    else this.history.unshift({ url, title, at: Date.now() });
    this.history = this.history.slice(0, HISTORY_MAX);
    localStorage.setItem("browser.history", JSON.stringify(this.history));
  }

  call(action, args) {
    try { return herdr.nativeViewCall(ID, action, args ?? {}); }
    catch (e) { this.error = String(e?.message ?? e); return null; }
  }

  go(text, cx) {
    this.error = null;
    const url = text.trim();
    if (!url) return;
    this.setPanel(null);
    // The first address creates the view; later ones navigate inside it.
    if (!this.start) { this.start = url; cx.notify(); return; }
    this.call("browser.navigate", { url });
    this.call("focus");
    cx.notify();
  }

  // Menus are drawn by Herdr, so the page steps aside while one is open.
  setPanel(panel) {
    this.panel = panel;
    this.confirmClear = false;
    if (this.start) this.call("setVisible", { visible: panel === null });
    if (panel === "menu") this.zoom = this.call("state")?.content?.zoomLevel ?? this.zoom;
  }

  runFind(next, forward, cx) {
    const text = this.findInput.value();
    if (!text) { this.call("browser.stopFind"); this.find = null; }
    else this.call("browser.find", { text, next, forward });
    cx.notify();
  }

  icon(t, id, label, enabled, onClick, w = 28) {
    return div().id(id).w(w).h(26).flex_none().flex().items_center().justify_center().rounded(6).text_size(14)
      .text_color(enabled ? t.foreground : t.dim)
      .when(enabled, el => el.cursor_pointer().hover(s => s.bg(t.hover)).on_click((_e, cx) => { onClick(cx); cx.notify(); }))
      .child(label);
  }

  item(t, id, label, onClick, right) {
    return h_flex().id(id).h(32).px(12).gap(8).items_center().rounded(6).cursor_pointer().hover(s => s.bg(t.hover))
      .on_click((_e, cx) => { onClick(cx); cx.notify(); })
      .child(div().flex_1().min_w(0).truncate().child(label))
      .when(Boolean(right), el => el.child(div().text_color(t.dim).text_size(12).child(right)));
  }

  menu(t) {
    const chromium = this.engine === "chromium";
    const zoom = h_flex().h(36).px(12).gap(6).items_center()
      .child(div().flex_1().child("缩放"))
      .child(this.icon(t, "z-out", "−", chromium, () => { this.zoom = this.call("browser.zoomOut")?.zoomLevel ?? this.zoom; }))
      .child(div().w(52).text_center().text_size(12).child(`${zoomPercent(this.zoom)}%`))
      .child(this.icon(t, "z-in", "+", chromium, () => { this.zoom = this.call("browser.zoomIn")?.zoomLevel ?? this.zoom; }))
      .child(this.icon(t, "z-reset", "↺", chromium, () => { this.zoom = this.call("browser.zoomReset")?.zoomLevel ?? 0; }));
    const sep = () => div().h(1).my(4).bg(t.border);
    return v_flex().id("browser-menu").p(6).gap(1).text_size(13)
      .when(chromium && Boolean(this.start), el => el
        .child(this.item(t, "m-find", "在页面中查找", () => { this.setPanel(null); this.finding = true; this.findInput.focus(); }))
        .child(this.item(t, "m-print", "打印", () => { this.setPanel(null); this.call("browser.print"); }))
        .child(zoom)
        .child(sep()))
      .child(this.item(t, "m-downloads", "下载", () => this.setPanel("downloads"), this.downloads.size ? String(this.downloads.size) : ""))
      .child(this.item(t, "m-history", "历史记录", () => this.setPanel("history"), this.history.length ? String(this.history.length) : ""))
      .when(chromium && Boolean(this.start), el => el.child(this.item(t, "m-import", "导入 Chrome 登录…", () => { this.importState = null; this.setPanel("import"); })))
      .child(sep())
      .child(this.item(t, "m-clear", this.confirmClear ? "再点一次确认：清除登录状态、缓存和历史记录" : "清除浏览数据", () => {
        if (!this.confirmClear) { this.confirmClear = true; return; }
        if (this.start && chromium) this.call("browser.clearData");
        this.history = []; localStorage.removeItem("browser.history");
        this.confirmClear = false;
        this.setPanel(null);
      }));
  }

  importPanel(t) {
    const s = this.importState;
    const btn = (id, label, primary, onClick) => div().id(id).px(12).h(30).flex().items_center().rounded(6).cursor_pointer()
      .bg(primary ? t.accent : t.raised).text_color(primary ? (t.accent_foreground ?? t.background) : t.foreground)
      .hover(x => x.bg(t.hover)).on_click((_e, cx) => { onClick(); cx.notify(); }).child(label);
    const para = (text) => div().text_size(13).text_color(t.muted ?? t.foreground).child(text);
    let body;
    if (!s) {
      body = v_flex().gap(8)
        .child(para("会把你本机 Google Chrome（最近使用的那个资料）里的 Cookie 复制到这个侧边浏览器，这样大多数网站不用重新登录。"))
        .child(para("macOS 会弹出钥匙串授权窗口（\"Chrome Safe Storage\"），点\"允许\"才能读取。数据只在本机内复制，不上传、不写日志；不导入密码；不会改动 Chrome 本身。"))
        .child(para("同名的 Cookie 会被覆盖。导入后可用\"清除浏览数据\"全部删掉。"))
        .child(h_flex().gap(8).pt(4)
          .child(btn("imp-go", "开始导入", true, () => { this.importState = { running: true }; this.call("browser.importChromeCookies"); }))
          .child(btn("imp-cancel", "取消", false, () => this.setPanel(null))));
    } else if (s.running) {
      body = v_flex().gap(8).child(para("正在读取 Chrome 的 Cookie……如果弹出钥匙串窗口，请点\"允许\"。"));
    } else if (s.error) {
      body = v_flex().gap(8).child(div().text_size(13).text_color(t.blocked).child(`没有导入：${s.error}`))
        .child(h_flex().gap(8).child(btn("imp-retry", "重试", true, () => { this.importState = { running: true }; this.call("browser.importChromeCookies"); }))
          .child(btn("imp-close", "关闭", false, () => this.setPanel(null))));
    } else {
      const d = s.done;
      body = v_flex().gap(8)
        .child(para(`已从 Chrome 资料「${d.profile}」导入 ${d.sites} 个网站的 ${d.cookies} 条 Cookie（跳过已过期或无法解密的 ${d.skipped} 条）。`))
        .child(h_flex().gap(8).child(btn("imp-reload", "刷新当前页面", true, () => { this.setPanel(null); this.call("browser.reload"); }))
          .child(btn("imp-done", "完成", false, () => this.setPanel(null))));
    }
    return v_flex().id("browser-import").p(12).gap(10)
      .child(h_flex().items_center().child(div().flex_1().font_semibold().child("导入 Chrome 登录")).child(this.icon(t, "imp-x", "✕", true, () => this.setPanel(null))))
      .child(body);
  }

  downloadsPanel(t) {
    const list = [...this.downloads.values()].reverse();
    return v_flex().id("browser-downloads").p(8).gap(2).text_size(13)
      .child(h_flex().h(30).px(6).items_center().child(div().flex_1().font_semibold().child("下载")).child(this.icon(t, "d-close", "✕", true, () => this.setPanel(null))))
      .when(!list.length, el => el.child(div().p(8).text_color(t.dim).child("还没有下载。网页上点下载链接时，会先让你选择保存位置。")))
      .children(list.map((d) => h_flex().id(`dl-${d.id}`).h(34).px(6).gap(8).items_center()
        .child(div().flex_1().min_w(0).truncate().child(d.name || "下载"))
        .child(div().text_size(12).text_color(t.dim).child(d.canceled ? "已取消" : d.complete ? "完成" : `${Math.max(0, d.percent ?? 0)}%`))
        .when(Boolean(d.complete), el => el.child(this.icon(t, `dl-show-${d.id}`, "在访达中显示", true, () => this.call("browser.revealDownload", { id: d.id }), 96)))));
  }

  historyPanel(t) {
    return v_flex().id("browser-history").size_full().min_h(0).p(8).gap(1).text_size(13)
      .child(h_flex().h(30).px(6).items_center().child(div().flex_1().font_semibold().child("历史记录")).child(this.icon(t, "h-close", "✕", true, () => this.setPanel(null))))
      .when(!this.history.length, el => el.child(div().p(8).text_color(t.dim).child("还没有浏览记录。")))
      .child(v_flex().id("history-list").flex_1().min_h(0).overflow_y_scrollbar()
        .children(this.history.slice(0, 100).map((h, i) => v_flex().id(`h-${i}`).px(8).py(4).rounded(6).cursor_pointer().hover(s => s.bg(t.hover))
          .on_click((_e, cx) => { this.setPanel(null); this.call("browser.navigate", { url: h.url }); cx.notify(); })
          .child(div().truncate().child(h.title || h.url))
          .child(div().truncate().text_size(11).text_color(t.dim).child(h.url)))));
  }

  render(cx) {
    const t = herdr.theme();
    if (!hasNative()) {
      return v_flex().size_full().p(16).bg(t.background).text_color(t.dim).text_size(13)
        .child("当前 Herdr 版本不能在窗口内显示网页（需要 Herdr Preview Dev）。");
    }
    const bar = h_flex().h(40).flex_none().px(6).gap(2).items_center().border_b(1).border_color(t.border)
      .child(this.icon(t, "b-back", "←", this.canBack, () => this.call("browser.back")))
      .child(this.icon(t, "b-fwd", "→", this.canForward, () => this.call("browser.forward")))
      .child(this.icon(t, "b-reload", this.loading ? "✕" : "↻", Boolean(this.start), () => this.call(this.loading ? "browser.stop" : "browser.reload")))
      .child(div().flex_1().min_w(0).child(Input.new(this.input)))
      .child(this.icon(t, "b-menu", "⋯", true, () => this.setPanel(this.panel ? null : "menu")));
    const findBar = h_flex().h(36).flex_none().px(8).gap(4).items_center().border_b(1).border_color(t.border).text_size(12)
      .child(div().flex_1().min_w(0).child(Input.new(this.findInput)))
      .child(div().w(56).text_center().text_color(t.dim).child(this.find ? `${this.find.active}/${this.find.matches}` : ""))
      .child(this.icon(t, "f-prev", "↑", Boolean(this.find), (cx) => this.runFind(true, false, cx)))
      .child(this.icon(t, "f-next", "↓", Boolean(this.find), (cx) => this.runFind(true, true, cx)))
      .child(this.icon(t, "f-close", "✕", true, () => { this.finding = false; this.find = null; this.findInput.set_value(""); this.call("browser.stopFind"); }));
    const status = this.error
      ? div().px(10).py(3).flex_none().text_size(11).truncate().text_color(t.blocked).child(`打不开：${this.error}`)
      : null;
    const page = div().flex_1().min_h(0).relative()
      .when(!this.start && !this.panel, el => el.child(div().p(16).text_color(t.dim).text_size(13).child("在上面输入网址，回车打开")))
      .when(Boolean(this.start), el => el.child(herdr.nativeView.new(ID, { kind: this.engine, url: this.start, persist: true })
        .child(div().size_full().bg(t.background)
          .when(this.panel === "menu", el => el.child(this.menu(t)))
          .when(this.panel === "downloads", el => el.child(this.downloadsPanel(t)))
          .when(this.panel === "history", el => el.child(this.historyPanel(t)))
          .when(this.panel === "import", el => el.child(this.importPanel(t))))))
      .when(!this.start && Boolean(this.panel), el => el.child(div().size_full().bg(t.background)
        .when(this.panel === "menu", el => el.child(this.menu(t)))
        .when(this.panel === "downloads", el => el.child(this.downloadsPanel(t)))
        .when(this.panel === "history", el => el.child(this.historyPanel(t)))));
    return v_flex().id("browser").size_full().min_h(0).overflow_hidden().rounded(10).border_1().border_color(t.border).bg(t.background).text_color(t.foreground)
      .child(bar)
      .when(this.finding, el => el.child(findBar))
      .when(Boolean(status), el => el.child(status))
      .child(page);
  }
}
