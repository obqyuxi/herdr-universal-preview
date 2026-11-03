// browser (DD-up): a web page inside Herdr's side panel.
// Type an address or words to search, press Enter. Chromium when the host has
// it (DD-up with CEF), else WebKit. Logins are kept in this app's own profile,
// separate from Safari and Chrome.
//
// The ⋯ menu and its pages are small cards over the page: the host cuts a hole
// in the page where a card is (`setHoles`), so the page stays visible around it.
import { View, div } from "gpui-kit";
import { v_flex, h_flex, InputState, Input } from "gpui-base";
import * as herdr from "herdr";

const ID = "browser";
const HISTORY_MAX = 300;
const FREQUENT_MAX = 12;
const TILE_COLORS = ["#4f7cff", "#16a37f", "#e0703a", "#9b5de5", "#d6457a", "#2b9fbf", "#c99a1a", "#5f6b7a"];
const ENGINES = [
  { id: "google", name: "Google", url: "https://www.google.com/search?q=%s" },
  { id: "bing", name: "必应", url: "https://www.bing.com/search?q=%s" },
  { id: "baidu", name: "百度", url: "https://www.baidu.com/s?wd=%s" },
  { id: "duckduckgo", name: "DuckDuckGo", url: "https://duckduckgo.com/?q=%s" },
];
// Cards: where they sit over the page (right/top) and how big they are. The hole
// the host cuts uses the same numbers, so they must match what render draws.
const ROW = 34, SEP = 9, PAD = 6, ZOOM_ROW = 42;
const CARDS = {
  menu: { w: 300, h: PAD * 2 + 7 * ROW + ZOOM_ROW + 3 * SEP },
  downloads: { w: 340, h: 300 },
  history: { w: 360, h: 440 },
  import: { w: 340, h: 300 },
  clear: { w: 300, h: 168 },
  settings: { w: 300, h: 52 + 26 + ENGINES.length * ROW + PAD * 2 },
};
const CARD_RIGHT = 10, CARD_TOP = 6, CARD_RADIUS = 12;

const hasNative = () => typeof herdr.nativeView?.new === "function";
const engine = () => (herdr.nativeViewCapabilities?.().kinds ?? []).some(k => k.kind === "chromium") ? "chromium" : "web";
const zoomPercent = (level) => Math.round(Math.pow(1.2, level ?? 0) * 100);

// An address, or words to search for.
function looksLikeUrl(t) {
  if (/\s/.test(t)) return false;
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(t)) return true;
  if (/^(localhost|\d{1,3}(\.\d{1,3}){3}|\[[0-9a-f:]+\])(:\d+)?(\/|$)/i.test(t)) return true;
  return /^[^/?#]+\.[a-z][a-z0-9-]+(:\d+)?([/?#].*)?$/i.test(t);
}

// Sites from the history, most visited first (ties: most recent). Each opens the
// page visited most often on that site.
function frequentSites(history) {
  const sites = new Map();
  history.forEach((h, i) => {
    let host;
    try { const u = new URL(h.url); if (!/^https?:$/.test(u.protocol)) return; host = u.host.replace(/^www\./, ""); }
    catch { return; }
    const s = sites.get(host) ?? { host, count: 0, recent: i, pages: new Map() };
    s.count += 1;
    const pg = s.pages.get(h.url) ?? { url: h.url, title: h.title, count: 0, recent: i };
    pg.count += 1;
    if (!pg.title && h.title) pg.title = h.title;
    s.pages.set(h.url, pg);
    sites.set(host, s);
  });
  return [...sites.values()]
    .sort((a, b) => b.count - a.count || a.recent - b.recent)
    .slice(0, FREQUENT_MAX)
    .map((s) => {
      const top = [...s.pages.values()].sort((a, b) => b.count - a.count || a.recent - b.recent)[0];
      return { host: s.host, url: top.url, title: top.title || s.host };
    });
}

export default class Browser extends View {
  init(_props, cx) {
    // No default site: the first time it shows the frequent sites; later it reopens the last page.
    this.start = localStorage.getItem("browser.url") || null;
    this.history = JSON.parse(localStorage.getItem("browser.history") || "[]");
    this.searchEngine = localStorage.getItem("browser.searchEngine") || "google";
    this.downloads = new Map();
    this.panel = null;          // null | "home" | a key of CARDS
    this.finding = false;
    this.find = null;
    this.zoom = 0;
    this.error = null;
    this.loading = false;
    this.canBack = false;
    this.canForward = false;
    this.title = "";
    this.addressFocused = false;
    this.input = InputState.new({ placeholder: "搜索或输入网址", value: this.start ?? "" });
    this.input.on("submit", (_e, cx) => this.go(this.input.value(), cx));
    this.input.on("focus", (_e, cx) => { this.addressFocused = true; cx.notify(); });
    this.input.on("blur", (_e, cx) => { this.addressFocused = false; cx.notify(); });
    this.findInput = InputState.new({ placeholder: "查找" });
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
    if (event === "view.click" && this.panel && this.panel !== "home") this.setPanel(null);
    if (event === "load.start") { this.loading = true; this.error = null; }
    if (event === "load.commit" || event === "load.finish") {
      if (event === "load.finish") this.loading = false;
      if (d.url) {
        this.url = d.url;
        if (!this.addressFocused) this.input.set_value(d.url);
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
    if (event === "download.update") this.downloads.set(d.id, { ...(this.downloads.get(d.id) ?? {}), ...d });
    if (event === "import.done") this.importState = { done: d };
    if (event === "import.error") this.importState = { error: d.message || "导入失败" };
  }

  remember(url, title) {
    if (this.history[0]?.url === url) { this.history[0].title = title || this.history[0].title; }
    else this.history.unshift({ url, title, at: Date.now() });
    this.history = this.history.slice(0, HISTORY_MAX);
    localStorage.setItem("browser.history", JSON.stringify(this.history));
  }

  // Before the first address there is no page to talk to. A failed call is a
  // bug on our side, not something the person can act on: log it, don't show it.
  call(action, args) {
    if (!this.start) return null;
    return this.host(action, args);
  }

  host(action, args) {
    try { return herdr.nativeViewCall(ID, action, args ?? {}); }
    catch (e) { console.log(`browser: ${action}: ${e?.message ?? e}`); return null; }
  }

  toUrl(text) {
    if (looksLikeUrl(text)) return text;
    const e = ENGINES.find(x => x.id === this.searchEngine) ?? ENGINES[0];
    return e.url.replace("%s", encodeURIComponent(text));
  }

  go(text, cx) {
    this.error = null;
    const t = text.trim();
    if (!t) return;
    const url = this.toUrl(t);
    this.setPanel(null);
    // The first address creates the view; later ones navigate inside it.
    if (!this.start) { this.start = url; cx.notify(); return; }
    this.call("browser.navigate", { url });
    this.call("focus");
    cx.notify();
  }

  setPanel(panel) {
    this.panel = panel;
    // The frequent-sites page covers the whole page area; cards only their own spot.
    this.call("setVisible", { visible: panel !== "home" });
    const c = CARDS[panel];
    this.call("setHoles", { holes: c ? [{ right: CARD_RIGHT, y: CARD_TOP, w: c.w, h: c.h, radius: CARD_RADIUS }] : [] });
    if (panel === "menu") this.zoom = this.call("state")?.content?.zoomLevel ?? this.zoom;
  }

  runFind(next, forward, cx) {
    const text = this.findInput.value();
    if (!text) { this.call("browser.stopFind"); this.find = null; }
    else this.call("browser.find", { text, next, forward });
    cx.notify();
  }

  startImport() {
    this.importState = { running: true };
    // Works before the first page too: the host starts Chromium for it.
    const r = this.host("browser.importChromeCookies");
    if (r === null) this.importState = { error: "现在无法导入，请先打开任意一个网页再试" };
  }

  // ---- pieces ----

  icon(t, id, label, enabled, onClick, w = 30, h = 30) {
    return div().id(id).w(w).h(h).flex_none().flex().items_center().justify_center().rounded(h / 2).text_size(15)
      .text_color(enabled ? t.foreground : t.dim)
      .when(enabled, el => el.cursor_pointer().hover(s => s.bg(t.hover)).on_click((_e, cx) => { onClick(cx); cx.notify(); }))
      .child(label);
  }

  item(t, id, label, enabled, onClick, right) {
    return h_flex().id(id).h(ROW).flex_none().px(12).gap(8).items_center().rounded(8).text_size(14)
      .text_color(enabled ? t.foreground : t.dim)
      .when(enabled, el => el.cursor_pointer().hover(s => s.bg(t.hover)).on_click((_e, cx) => { onClick(cx); cx.notify(); }))
      .child(div().flex_1().min_w(0).truncate().child(label))
      .when(Boolean(right), el => el.child(div().text_color(t.dim).text_size(12).child(right)));
  }

  sep(t) {
    return div().h(SEP).flex_none().flex().items_center().px(12).child(div().w_full().h(1).bg(t.border));
  }

  card(t, name, body) {
    const c = CARDS[name];
    return v_flex().id(`card-${name}`).absolute().top(CARD_TOP).right(CARD_RIGHT).w(c.w).h(c.h)
      .overflow_hidden().rounded(CARD_RADIUS).border_1().border_color(t.border).bg(t.background).text_color(t.foreground)
      .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
      .child(body);
  }

  cardHeader(t, title) {
    return h_flex().h(44).flex_none().px(8).gap(4).items_center()
      .child(this.icon(t, `back-${title}`, "‹", true, () => this.setPanel("menu"), 28, 28))
      .child(div().flex_1().text_size(14).font_semibold().child(title))
      .child(this.icon(t, `close-${title}`, "✕", true, () => this.setPanel(null), 28, 28));
  }

  menu(t) {
    const page = Boolean(this.start);
    const chromium = this.engine === "chromium";
    const z = chromium && page;
    const zoom = h_flex().h(ZOOM_ROW).flex_none().px(12).gap(8).items_center().text_size(14)
      .child(div().flex_1().child("缩放"))
      .child(h_flex().h(30).items_center().rounded(8).border_1().border_color(t.border)
        .child(this.icon(t, "z-out", "−", z, () => { this.zoom = this.call("browser.zoomOut")?.zoomLevel ?? this.zoom; }, 32, 28))
        .child(div().w(54).text_center().text_size(13).text_color(z ? t.foreground : t.dim).child(`${zoomPercent(this.zoom)}%`))
        .child(this.icon(t, "z-in", "+", z, () => { this.zoom = this.call("browser.zoomIn")?.zoomLevel ?? this.zoom; }, 32, 28)))
      .child(this.icon(t, "z-reset", "↺", z && this.zoom !== 0, () => { this.zoom = this.call("browser.zoomReset")?.zoomLevel ?? 0; }, 28, 28));
    return v_flex().p(PAD)
      .child(this.item(t, "m-find", "在页面中查找", z, () => { this.setPanel(null); this.finding = true; this.findInput.focus(); }))
      .child(this.item(t, "m-print", "打印", z, () => { this.setPanel(null); this.call("browser.print"); }))
      .child(this.sep(t))
      .child(zoom)
      .child(this.sep(t))
      .child(this.item(t, "m-import", "导入 Cookie…", chromium, () => { this.importState = null; this.setPanel("import"); }))
      .child(this.item(t, "m-downloads", "下载", true, () => this.setPanel("downloads"), this.downloads.size ? String(this.downloads.size) : ""))
      .child(this.item(t, "m-history", "历史记录", true, () => this.setPanel("history")))
      .child(this.item(t, "m-clear", "清除浏览数据", true, () => this.setPanel("clear")))
      .child(this.sep(t))
      .child(this.item(t, "m-settings", "浏览器设置", true, () => this.setPanel("settings")));
  }

  button(t, id, label, primary, onClick) {
    return div().id(id).px(14).h(30).flex().items_center().rounded(8).cursor_pointer().text_size(13)
      .bg(primary ? t.accent : t.raised).text_color(primary ? (t.accent_foreground ?? t.background) : t.foreground)
      .hover(x => x.opacity(0.85)).on_click((_e, cx) => { onClick(); cx.notify(); }).child(label);
  }

  importPanel(t) {
    const s = this.importState;
    const para = (text) => div().text_size(13).text_color(t.muted ?? t.foreground).child(text);
    let body;
    if (!s) {
      body = v_flex().gap(8)
        .child(para("把本机 Google Chrome 的 Cookie 复制到这个浏览器，大多数网站就不用重新登录。"))
        .child(para("macOS 会询问能否读取钥匙串里的 \"Chrome Safe Storage\"，点\"允许\"即可。只在本机复制，不导入密码，不改动 Chrome。"))
        .child(h_flex().gap(8).pt(6)
          .child(this.button(t, "imp-go", "导入", true, () => this.startImport()))
          .child(this.button(t, "imp-cancel", "取消", false, () => this.setPanel(null))));
    } else if (s.running) {
      body = para("正在读取 Chrome 的 Cookie……");
    } else if (s.error) {
      body = v_flex().gap(8).child(div().text_size(13).text_color(t.blocked).child(`没有导入：${s.error}`))
        .child(h_flex().gap(8).pt(6).child(this.button(t, "imp-retry", "重试", true, () => this.startImport()))
          .child(this.button(t, "imp-close", "关闭", false, () => this.setPanel(null))));
    } else {
      const d = s.done;
      body = v_flex().gap(8)
        .child(para(`已导入 ${d.sites} 个网站的 ${d.cookies} 条 Cookie（来自 Chrome 资料「${d.profile}」）。`))
        .child(h_flex().gap(8).pt(6)
          .when(Boolean(this.start), el => el.child(this.button(t, "imp-reload", "刷新页面", true, () => { this.setPanel(null); this.call("browser.reload"); })))
          .child(this.button(t, "imp-done", "完成", !this.start, () => this.setPanel(null))));
    }
    return v_flex().child(this.cardHeader(t, "导入 Cookie")).child(v_flex().px(16).pb(12).child(body));
  }

  clearPanel(t) {
    return v_flex().child(this.cardHeader(t, "清除浏览数据"))
      .child(v_flex().px(16).gap(10)
        .child(div().text_size(13).text_color(t.muted ?? t.foreground).child("删除这个浏览器的登录状态、缓存和历史记录。"))
        .child(h_flex().gap(8).pt(4)
          .child(this.button(t, "clear-go", "清除", true, () => {
            if (this.engine === "chromium") this.call("browser.clearData");
            this.history = []; localStorage.removeItem("browser.history");
            this.setPanel(null);
          }))
          .child(this.button(t, "clear-cancel", "取消", false, () => this.setPanel(null)))));
  }

  settingsPanel(t) {
    return v_flex().child(this.cardHeader(t, "浏览器设置"))
      .child(div().h(26).px(16).text_size(12).text_color(t.dim).child("搜索引擎"))
      .child(v_flex().px(PAD).children(ENGINES.map((e) => this.item(t, `se-${e.id}`, e.name, true, () => {
        this.searchEngine = e.id; localStorage.setItem("browser.searchEngine", e.id);
      }, e.id === this.searchEngine ? "✓" : ""))));
  }

  downloadsPanel(t) {
    const list = [...this.downloads.values()].reverse();
    return v_flex().size_full().child(this.cardHeader(t, "下载"))
      .child(v_flex().id("dl-list").flex_1().min_h(0).px(PAD).pb(PAD).overflow_y_scrollbar().text_size(13)
        .when(!list.length, el => el.child(div().px(10).py(6).text_color(t.dim).child("还没有下载")))
        .children(list.map((d) => h_flex().id(`dl-${d.id}`).h(ROW).px(10).gap(8).items_center()
          .child(div().flex_1().min_w(0).truncate().child(d.name || "下载"))
          .child(div().text_size(12).text_color(t.dim).child(d.canceled ? "已取消" : d.complete ? "完成" : `${Math.max(0, d.percent ?? 0)}%`))
          .when(Boolean(d.complete), el => el.child(this.icon(t, `dl-show-${d.id}`, "在访达中显示", true, () => this.call("browser.revealDownload", { id: d.id }), 96, 26))))));
  }

  historyPanel(t) {
    return v_flex().size_full().child(this.cardHeader(t, "历史记录"))
      .child(v_flex().id("history-list").flex_1().min_h(0).px(PAD).pb(PAD).overflow_y_scrollbar().text_size(13)
        .when(!this.history.length, el => el.child(div().px(10).py(6).text_color(t.dim).child("还没有浏览记录")))
        .children(this.history.slice(0, 100).map((h, i) => v_flex().id(`h-${i}`).px(10).py(5).rounded(8).cursor_pointer().hover(s => s.bg(t.hover))
          .on_click((_e, cx) => { this.input.set_value(h.url); this.go(h.url, cx); })
          .child(div().truncate().child(h.title || h.url))
          .child(div().truncate().text_size(11).text_color(t.dim).child(h.url)))));
  }

  // The new-tab page: frequent sites.
  homePanel(t) {
    const sites = frequentSites(this.history);
    const tile = (s, i) => {
      const color = TILE_COLORS[[...s.host].reduce((a, c) => a + c.charCodeAt(0), 0) % TILE_COLORS.length];
      return v_flex().id(`site-${i}`).w(120).px(8).py(10).gap(8).items_center().rounded(12).cursor_pointer().hover(x => x.bg(t.hover))
        .on_click((_e, cx) => { this.input.set_value(s.url); this.go(s.url, cx); })
        .child(div().w(52).h(52).rounded(14).bg(color).flex().items_center().justify_center()
          .text_size(22).font_semibold().text_color("#ffffff").child(s.host.charAt(0).toUpperCase()))
        .child(div().w_full().text_center().text_size(13).truncate().child(s.title));
    };
    return v_flex().id("browser-home").size_full().px(24).py(20).gap(12).bg(t.background)
      .when(sites.length > 0, el => el
        .child(div().text_size(15).font_semibold().child("常用网站"))
        .child(div().flex().flex_wrap().content_start().gap(8).children(sites.map(tile))));
  }

  cardFor(t) {
    const p = this.panel;
    if (!CARDS[p]) return null;
    const body = p === "menu" ? this.menu(t) : p === "downloads" ? this.downloadsPanel(t) : p === "history" ? this.historyPanel(t)
      : p === "import" ? this.importPanel(t) : p === "clear" ? this.clearPanel(t) : this.settingsPanel(t);
    return this.card(t, p, body);
  }

  render(cx) {
    const t = herdr.theme();
    if (!hasNative()) {
      return v_flex().size_full().p(16).bg(t.background).text_color(t.dim).text_size(13)
        .child("这个版本的 Herdr 不能在窗口里显示网页，需要 DD-up。");
    }
    const nav = h_flex().h(36).flex_none().px(3).items_center().rounded(18).border_1().border_color(t.border)
      .child(this.icon(t, "b-back", "←", this.canBack, () => this.call("browser.back")))
      .child(this.icon(t, "b-fwd", "→", this.canForward, () => this.call("browser.forward")))
      .child(div().w(1).h(16).mx(3).bg(t.border))
      .child(this.icon(t, "b-reload", this.loading ? "✕" : "↻", Boolean(this.start), () => this.call(this.loading ? "browser.stop" : "browser.reload")))
      .child(this.icon(t, "b-home", "⌂", Boolean(this.start), () => this.setPanel(this.panel === "home" ? null : "home")));
    const address = div().flex_1().min_w(0).h(36).px(14).flex().items_center().rounded(18).border_1()
      .border_color(this.addressFocused ? t.accent : t.border).cursor_text()
      .child(Input.new(this.input).flex_1().h_full().text_size(13).text_color(t.foreground));
    const more = div().id("b-menu").w(36).h(36).flex_none().flex().items_center().justify_center().rounded(18).border_1()
      .border_color(t.border).text_size(16).cursor_pointer().hover(s => s.bg(t.hover)).when(CARDS[this.panel] != null, el => el.bg(t.hover))
      .on_click((_e, cx) => { this.setPanel(CARDS[this.panel] ? null : "menu"); cx.notify(); })
      .child("⋯");
    const bar = h_flex().h(52).flex_none().px(10).gap(8).items_center().border_b(1).border_color(t.border)
      .child(nav).child(address).child(more);
    const findBar = h_flex().h(38).flex_none().px(10).gap(4).items_center().border_b(1).border_color(t.border).text_size(12)
      .child(div().flex_1().min_w(0).child(Input.new(this.findInput)))
      .child(div().w(56).text_center().text_color(t.dim).child(this.find ? `${this.find.active}/${this.find.matches}` : ""))
      .child(this.icon(t, "f-prev", "↑", Boolean(this.find), (cx) => this.runFind(true, false, cx), 26, 26))
      .child(this.icon(t, "f-next", "↓", Boolean(this.find), (cx) => this.runFind(true, true, cx), 26, 26))
      .child(this.icon(t, "f-close", "✕", true, () => { this.finding = false; this.find = null; this.findInput.set_value(""); this.call("browser.stopFind"); }, 26, 26));
    const status = this.error
      ? div().px(12).py(3).flex_none().text_size(11).truncate().text_color(t.blocked).child(`打不开：${this.error}`)
      : null;
    const card = this.cardFor(t);
    const overlay = div().size_full().relative()
      .when(this.panel === "home" || !this.start, el => el.child(this.homePanel(t)))
      .when(Boolean(card), el => el.child(card));
    const page = div().flex_1().min_h(0).relative()
      .when(Boolean(this.start), el => el.child(herdr.nativeView.new(ID, { kind: this.engine, url: this.start, persist: true }).child(overlay)))
      .when(!this.start, el => el.child(overlay));
    return v_flex().id("browser").size_full().min_h(0).overflow_hidden().rounded(10).border_1().border_color(t.border).bg(t.background).text_color(t.foreground)
      .child(bar)
      .when(this.finding, el => el.child(findBar))
      .when(Boolean(status), el => el.child(status))
      .child(page);
  }
}
