// browser (DD-up): a web page inside Herdr's side panel.
// Type an address or words to search, press Enter. Chromium when the host has
// it (DD-up with CEF), else WebKit. Logins are kept in this app's own profile,
// separate from Safari and Chrome.
//
// The ⋯ menu and its pages are small cards over the page: the host cuts a hole
// in the page where a card is (`setHoles`), so the page stays visible around it.
import { View, div, svg, image } from "gpui-kit";
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
// the host cuts uses the same numbers, so every height below is the sum of the
// fixed rows render draws (Herdr's own menu metrics: 32px rows, 13px text).
const ROW = 32, SEP = 9, PAD = 5, ZOOM_ROW = 40, HEADER = 44, LINE = 20, BUTTONS = 30, BODY_PAD = 16;
const textCard = (lines) => HEADER + lines * LINE + (lines - 1) * 4 + 14 + BUTTONS + BODY_PAD;
const HISTORY_ROW = 46, DOWNLOAD_ROW = 42, LIST_MAX = 8;
const listCard = (rows, rowH) => HEADER + Math.max(1, Math.min(rows, LIST_MAX)) * rowH + PAD;
const CARDS = {
  menu: { w: 280, h: PAD * 2 + 7 * ROW + ZOOM_ROW + 3 * SEP },
  downloads: { w: 340, h: listCard(1, DOWNLOAD_ROW) },
  history: { w: 360, h: listCard(1, HISTORY_ROW) },
  import: { w: 340, h: textCard(3) },
  clear: { w: 300, h: textCard(1) },
  settings: { w: 280, h: HEADER + 24 + ENGINES.length * ROW + PAD },
};
const CARD_RIGHT = 10, CARD_TOP = 6, CARD_RADIUS = 10;
const ICONS = "mods/browser/icons";

const hasNative = () => typeof herdr.nativeView?.new === "function";
const engine = () => (herdr.nativeViewCapabilities?.().kinds ?? []).some(k => k.kind === "chromium") ? "chromium" : "web";
const zoomPercent = (level) => Math.round(Math.pow(1.2, level ?? 0) * 100);

// Same rule as the host's favicon file names: plain host, no "www.", ":" -> "_".
const iconKey = (url) => { try { const u = new URL(url); return /^https?:$/.test(u.protocol) ? u.host.toLowerCase().replace(/^www\./, "").replace(":", "_") : null; } catch { return null; } };

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
    this.favicons = JSON.parse(localStorage.getItem("browser.favicons") || "{}");  // host -> path in the app home
    this.progress = 0;
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
    // Key commands (keys.json `browser.<command>`): menu, address, find, home,
    // and `panel:<card>` to open one card directly.
    cx.spawn(async (cx) => {
      for (;;) {
        const c = String(await herdr.nextCommand());
        const [name, arg] = c.split(":");
        if (name === "menu") this.setPanel(CARDS[this.panel] ? null : "menu");
        if (name === "panel" && (CARDS[arg] || arg === "home")) { if (arg === "import") this.importState = null; this.setPanel(arg); }
        if (name === "close") this.setPanel(null);
        if (name === "address" || name === "find") this.onKey(name);
        if (name === "home" && this.start) this.setPanel(this.panel === "home" ? null : "home");
        cx.notify();
      }
    });
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
    if (event === "key") this.onKey(d.name);
    if (event === "load.progress") this.progress = d.progress ?? 0;
    if (event === "favicon" && d.host && d.path && this.favicons[d.host] !== d.path) {
      this.favicons[d.host] = d.path;
      localStorage.setItem("browser.favicons", JSON.stringify(this.favicons));
    }
    if (event === "load.start") { this.loading = true; this.error = null; this.progress = 0; }
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
    if (event === "download.start") { this.downloads.set(d.id, { id: d.id, name: d.name, percent: 0, complete: false }); if (this.panel === "downloads") this.setPanel("downloads"); }
    if (event === "download.update") this.downloads.set(d.id, { ...(this.downloads.get(d.id) ?? {}), ...d });
    if (event === "import.done") this.importState = { done: d };
    if (event === "import.error") this.importState = { error: d.message || "导入失败" };
  }

  // Shortcuts: from the page (the host reports them) or from Herdr's side of the panel.
  onKey(name) {
    const toHerdr = () => this.call("blur");  // the page lets go of the keyboard first
    if (name === "address") { toHerdr(); this.setPanel(null); this.input.focus(); }
    else if (name === "find" && this.start) { toHerdr(); this.setPanel(null); this.finding = true; this.findInput.focus(); }
    else if (name === "escape") {
      if (this.panel) this.setPanel(null);
      else if (this.finding) { this.finding = false; this.find = null; this.findInput.set_value(""); this.call("browser.stopFind"); }
    }
    else if (name === "reload") this.call("browser.reload");
    else if (name === "back") this.call("browser.back");
    else if (name === "forward") this.call("browser.forward");
    else if (name === "zoomIn" || name === "zoomOut" || name === "zoomReset") this.zoom = this.call(`browser.${name}`)?.zoomLevel ?? this.zoom;
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
    const c = this.cardSize(panel);
    this.call("setHoles", { holes: c ? [{ right: CARD_RIGHT, y: CARD_TOP, w: c.w, h: c.h, radius: CARD_RADIUS }] : [] });
    if (panel === "menu") this.zoom = this.call("state")?.content?.zoomLevel ?? this.zoom;
  }

  historyRows() {
    const seen = new Set();
    return this.history.filter((h) => !seen.has(h.url) && seen.add(h.url)).slice(0, 100);
  }

  // History and downloads grow with their lists (up to LIST_MAX rows, then scroll).
  cardSize(name) {
    const c = CARDS[name];
    if (!c) return null;
    if (name === "history") return { w: c.w, h: listCard(this.historyRows().length, HISTORY_ROW) };
    if (name === "downloads") return { w: c.w, h: listCard(this.downloads.size, DOWNLOAD_ROW) };
    return c;
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

  glyph(name, size, color) {
    return svg(`${ICONS}/${name}.svg`).size(size).flex_none().text_color(color);
  }

  // A round icon button; disabled ones stay in place, dimmed.
  icon(t, id, name, enabled, onClick, size = 30, iconSize = 16) {
    return div().id(id).w(size).h(size).flex_none().flex().items_center().justify_center().rounded(size / 2)
      .when(enabled, el => el.cursor_pointer().hover(s => s.bg(t.hover)).on_click((_e, cx) => { onClick(cx); cx.notify(); }))
      .child(this.glyph(name, iconSize, enabled ? t.foreground : t.dim));
  }

  item(t, id, label, enabled, onClick, right) {
    return h_flex().id(id).h(ROW).flex_none().px(10).gap(8).items_center().rounded(7).text_size(13)
      .text_color(enabled ? t.foreground : t.dim)
      .when(enabled, el => el.cursor_pointer().hover(s => s.bg(t.hover)).on_click((_e, cx) => { onClick(cx); cx.notify(); }))
      .child(div().flex_1().min_w(0).truncate().child(label))
      .when(right != null, el => el.child(right));
  }

  sep(t) {
    return div().h(SEP).flex_none().flex().items_center().px(10).child(div().w_full().h(1).bg(t.border));
  }

  card(t, name, body) {
    const c = this.cardSize(name);
    return v_flex().id(`card-${name}`).absolute().top(CARD_TOP).right(CARD_RIGHT).w(c.w).h(c.h)
      .overflow_hidden().rounded(CARD_RADIUS).border_1().border_color(t.border).bg(t.popover ?? t.background).text_color(t.foreground)
      .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
      .child(body);
  }

  cardHeader(t, title) {
    return h_flex().h(HEADER).flex_none().pl(6).pr(8).gap(2).items_center()
      .child(this.icon(t, `back-${title}`, "chevron-left", true, () => this.setPanel("menu"), 28, 16))
      .child(div().flex_1().text_size(13).font_semibold().child(title))
      .child(this.icon(t, `close-${title}`, "x", true, () => this.setPanel(null), 28, 15));
  }

  // Short lines, one sentence each: GPUI wraps CJK anywhere, so a long line
  // could start with "，" or leave a lone "。".
  lines(t, list) {
    return v_flex().gap(4).children(list.map((l) => div().h(LINE).text_size(13).text_color(t.muted ?? t.foreground).truncate().child(l)));
  }

  textCardBody(t, list, buttons) {
    return v_flex().px(BODY_PAD).child(this.lines(t, list)).child(h_flex().gap(8).pt(14).child(buttons[0]).when(Boolean(buttons[1]), el => el.child(buttons[1])));
  }

  menu(t) {
    const page = Boolean(this.start);
    const chromium = this.engine === "chromium";
    const z = chromium && page;
    const zoom = h_flex().h(ZOOM_ROW).flex_none().px(10).gap(6).items_center().text_size(13)
      .text_color(z ? t.foreground : t.dim)
      .child(div().flex_1().child("缩放"))
      .child(h_flex().h(28).items_center().rounded(7).border_1().border_color(t.border)
        .child(this.icon(t, "z-out", "minus", z, () => { this.zoom = this.call("browser.zoomOut")?.zoomLevel ?? this.zoom; }, 28, 14))
        .child(div().w(48).text_center().text_size(12).child(`${zoomPercent(this.zoom)}%`))
        .child(this.icon(t, "z-in", "plus", z, () => { this.zoom = this.call("browser.zoomIn")?.zoomLevel ?? this.zoom; }, 28, 14)))
      .child(this.icon(t, "z-reset", "rotate-ccw", z && this.zoom !== 0, () => { this.zoom = this.call("browser.zoomReset")?.zoomLevel ?? 0; }, 28, 14));
    const count = (n) => n ? div().text_size(12).text_color(t.dim).child(String(n)) : null;
    return v_flex().p(PAD)
      .child(this.item(t, "m-find", "在页面中查找", z, () => { this.setPanel(null); this.finding = true; this.findInput.focus(); }))
      .child(this.item(t, "m-print", "打印", z, () => { this.setPanel(null); this.call("browser.print"); }))
      .child(this.sep(t))
      .child(zoom)
      .child(this.sep(t))
      .child(this.item(t, "m-import", "导入 Cookie…", chromium, () => { this.importState = null; this.setPanel("import"); }))
      .child(this.item(t, "m-downloads", "下载", true, () => this.setPanel("downloads"), count(this.downloads.size)))
      .child(this.item(t, "m-history", "历史记录", true, () => this.setPanel("history")))
      .child(this.item(t, "m-clear", "清除浏览数据…", true, () => this.setPanel("clear")))
      .child(this.sep(t))
      .child(this.item(t, "m-settings", "浏览器设置", true, () => this.setPanel("settings")));
  }

  button(t, id, label, primary, onClick) {
    return div().id(id).px(14).h(BUTTONS).flex().items_center().rounded(7).cursor_pointer().text_size(13)
      .when(primary, el => el.bg(t.accent).text_color(t.solid_foreground ?? "#ffffff"))
      .when(!primary, el => el.border_1().border_color(t.border).text_color(t.foreground))
      .hover(x => x.opacity(0.88)).on_click((_e, cx) => { onClick(); cx.notify(); }).child(label);
  }

  importPanel(t) {
    const s = this.importState;
    let body;
    if (!s) {
      body = this.textCardBody(t, [
        "复制本机 Chrome 的 Cookie 到这个浏览器。",
        "macOS 会请求钥匙串授权，点“允许”即可。",
        "只在本机复制，不导入密码，不改动 Chrome。",
      ], [this.button(t, "imp-go", "导入", true, () => this.startImport()), this.button(t, "imp-cancel", "取消", false, () => this.setPanel(null))]);
    } else if (s.running) {
      body = v_flex().px(BODY_PAD).child(this.lines(t, ["正在读取 Chrome 的 Cookie……"]));
    } else if (s.error) {
      body = v_flex().px(BODY_PAD).gap(4)
        .child(div().text_size(13).text_color(t.blocked).child("没有导入"))
        .child(div().text_size(12).text_color(t.dim).truncate().child(s.error))
        .child(h_flex().gap(8).pt(10).child(this.button(t, "imp-retry", "重试", true, () => this.startImport()))
          .child(this.button(t, "imp-close", "关闭", false, () => this.setPanel(null))));
    } else {
      const d = s.done;
      body = this.textCardBody(t, [
        `已导入 ${d.sites} 个网站的 ${d.cookies} 条 Cookie。`,
        `来自 Chrome 资料「${d.profile}」。`,
        "已登录的网站刷新后即可使用。",
      ], this.start
        ? [this.button(t, "imp-reload", "刷新页面", true, () => { this.setPanel(null); this.call("browser.reload"); }), this.button(t, "imp-done", "完成", false, () => this.setPanel(null))]
        : [this.button(t, "imp-done", "完成", true, () => this.setPanel(null))]);
    }
    return v_flex().child(this.cardHeader(t, "导入 Cookie")).child(body);
  }

  clearPanel(t) {
    return v_flex().child(this.cardHeader(t, "清除浏览数据"))
      .child(this.textCardBody(t, ["删除登录状态、缓存和历史记录。"], [
        this.button(t, "clear-go", "清除", true, () => {
          if (this.engine === "chromium") this.call("browser.clearData");
          this.history = []; localStorage.removeItem("browser.history");
          this.setPanel(null);
        }),
        this.button(t, "clear-cancel", "取消", false, () => this.setPanel(null)),
      ]));
  }

  settingsPanel(t) {
    return v_flex().child(this.cardHeader(t, "浏览器设置"))
      .child(div().h(24).px(BODY_PAD).text_size(12).text_color(t.dim).child("地址栏搜索使用"))
      .child(v_flex().px(PAD).children(ENGINES.map((e) => this.item(t, `se-${e.id}`, e.name, true, () => {
        this.searchEngine = e.id; localStorage.setItem("browser.searchEngine", e.id);
      }, e.id === this.searchEngine ? this.glyph("check", 15, t.accent) : null))));
  }

  downloadsPanel(t) {
    const list = [...this.downloads.values()].reverse();
    return v_flex().size_full().child(this.cardHeader(t, "下载"))
      .child(v_flex().id("dl-list").flex_1().min_h(0).px(PAD).pb(PAD).overflow_y_scrollbar().text_size(13)
        .when(!list.length, el => el.child(div().h(DOWNLOAD_ROW).px(10).flex().items_center().text_color(t.dim).child("还没有下载")))
        .children(list.map((d) => h_flex().id(`dl-${d.id}`).h(DOWNLOAD_ROW).flex_none().px(10).gap(8).items_center().rounded(7)
          .child(v_flex().flex_1().min_w(0)
            .child(div().truncate().child(d.name || "下载"))
            .child(div().text_size(11).text_color(t.dim).child(d.canceled ? "已取消" : d.complete ? "已完成" : `${Math.max(0, d.percent ?? 0)}%`)))
          .when(Boolean(d.complete), el => el.child(this.icon(t, `dl-show-${d.id}`, "folder-open", true, () => this.call("browser.revealDownload", { id: d.id }), 28, 15))))));
  }

  historyPanel(t) {
    const rows = this.historyRows();  // one row per page, newest first
    return v_flex().size_full().child(this.cardHeader(t, "历史记录"))
      .child(v_flex().id("history-list").flex_1().min_h(0).px(PAD).pb(PAD).overflow_y_scrollbar().text_size(13)
        .when(!rows.length, el => el.child(div().h(HISTORY_ROW).px(10).flex().items_center().text_color(t.dim).child("还没有浏览记录")))
        .children(rows.map((h, i) => h_flex().id(`h-${i}`).h(HISTORY_ROW).flex_none().gap(10).items_center().px(10).rounded(7).cursor_pointer().hover(s => s.bg(t.hover))
          .on_click((_e, cx) => { this.input.set_value(h.url); this.go(h.url, cx); })
          .child(this.siteIcon(t, h.url, (iconKey(h.url) ?? "?").replace(/_\d+$/, ""), 24))
          .child(v_flex().flex_1().min_w(0)
            .child(div().truncate().child(h.title || h.url))
            .child(div().truncate().text_size(11).text_color(t.dim).child(h.url))))));
  }

  // The site's own icon once the host has saved it, else its initial on a color.
  siteIcon(t, url, host, size) {
    const path = this.favicons[iconKey(url)];
    if (path) return div().w(size).h(size).flex_none().rounded(size / 4).bg(t.chrome ?? t.hover).flex().items_center().justify_center()
      .child(image(path).size(Math.round(size * 0.58)));
    const color = TILE_COLORS[[...host].reduce((a, c) => a + c.charCodeAt(0), 0) % TILE_COLORS.length];
    return div().w(size).h(size).flex_none().rounded(size / 4).bg(color).flex().items_center().justify_center()
      .text_size(Math.round(size * 0.42)).font_semibold().text_color("#ffffff").child(host.charAt(0).toUpperCase());
  }

  // The new-tab page: frequent sites.
  homePanel(t) {
    const sites = frequentSites(this.history);
    const tile = (s, i) => {
      return v_flex().id(`site-${i}`).w(112).px(6).py(10).gap(8).items_center().rounded(10).cursor_pointer().hover(x => x.bg(t.hover))
        .on_click((_e, cx) => { this.input.set_value(s.url); this.go(s.url, cx); })
        .child(this.siteIcon(t, s.url, s.host, 48))
        .child(div().w_full().text_center().text_size(12).truncate().child(s.title));
    };
    return v_flex().id("browser-home").size_full().px(20).pt(24).pb(16).gap(10).bg(t.background)
      .when(sites.length > 0, el => el
        .child(div().px(6).text_size(13).font_semibold().text_color(t.muted ?? t.foreground).child("常用网站"))
        .child(div().flex().flex_wrap().content_start().children(sites.map(tile))));
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
    const page = Boolean(this.start);
    const nav = h_flex().h(34).flex_none().px(2).items_center().rounded(17).border_1().border_color(t.border)
      .child(this.icon(t, "b-back", "arrow-left", this.canBack, () => this.call("browser.back")))
      .child(this.icon(t, "b-fwd", "arrow-right", this.canForward, () => this.call("browser.forward")))
      .child(div().w(1).h(14).mx(2).bg(t.border))
      .child(this.icon(t, "b-reload", this.loading ? "x" : "rotate-cw", page, () => this.call(this.loading ? "browser.stop" : "browser.reload"), 30, this.loading ? 16 : 15))
      .child(this.icon(t, "b-home", "house", page, () => this.setPanel(this.panel === "home" ? null : "home"), 30, 15));
    const address = div().flex_1().min_w(0).h(34).px(14).flex().items_center().rounded(17).border_1()
      .border_color(this.addressFocused ? t.accent : (t.chrome ?? t.border)).bg(this.addressFocused ? t.background : (t.chrome ?? t.background)).cursor_text()
      .child(Input.new(this.input).flex_1().h_full().text_size(13).text_color(t.foreground));
    const open = CARDS[this.panel] != null;
    const more = div().id("b-menu").w(34).h(34).flex_none().flex().items_center().justify_center().rounded(17).border_1()
      .border_color(t.border).cursor_pointer().hover(s => s.bg(t.hover)).when(open, el => el.bg(t.hover))
      .on_click((_e, cx) => { this.setPanel(open ? null : "menu"); cx.notify(); })
      .child(this.glyph("ellipsis", 16, t.foreground));
    const loadingBar = this.loading && page
      ? div().absolute().left(0).bottom(-1).h(2).w(`${Math.round(Math.max(0.08, Math.min(1, this.progress)) * 100)}%`).bg(t.accent)
      : null;
    const bar = h_flex().h(50).flex_none().relative().px(8).gap(6).items_center().border_b(1).border_color(t.border)
      .child(nav).child(address).child(more)
      .when(Boolean(loadingBar), el => el.child(loadingBar));
    const findBar = h_flex().h(40).flex_none().px(10).gap(4).items_center().border_b(1).border_color(t.border).text_size(12)
      .child(this.glyph("search", 14, t.dim))
      .child(div().flex_1().min_w(0).pl(4).child(Input.new(this.findInput).text_size(13)))
      .child(div().min_w(48).text_center().text_size(12).text_color(t.dim).child(this.find ? `${this.find.active}/${this.find.matches}` : ""))
      .child(this.icon(t, "f-prev", "chevron-up", Boolean(this.find?.matches), (cx) => this.runFind(true, false, cx), 26, 15))
      .child(this.icon(t, "f-next", "chevron-down", Boolean(this.find?.matches), (cx) => this.runFind(true, true, cx), 26, 15))
      .child(this.icon(t, "f-close", "x", true, () => { this.finding = false; this.find = null; this.findInput.set_value(""); this.call("browser.stopFind"); }, 26, 14));
    const status = this.error
      ? div().px(12).py(4).flex_none().text_size(12).truncate().text_color(t.blocked).border_b(1).border_color(t.border).child(`打不开：${this.error}`)
      : null;
    const card = this.cardFor(t);
    const overlay = div().size_full().relative()
      .when(this.panel === "home" || !page, el => el.child(this.homePanel(t)))
      .when(Boolean(card), el => el.child(card));
    const body = div().flex_1().min_h(0).relative()
      .when(page, el => el.child(herdr.nativeView.new(ID, { kind: this.engine, url: this.start, persist: true }).child(overlay)))
      .when(!page, el => el.child(overlay));
    return v_flex().id("browser").size_full().min_h(0)
      .on_key_down((e, cx) => {
        const cmd = e.modifiers?.platform && !e.modifiers.control && !e.modifiers.alt;
        if (cmd && e.key === "l") this.onKey("address");
        else if (cmd && e.key === "f") this.onKey("find");
        else if (e.key === "escape" && (this.panel || this.finding)) this.onKey("escape");
        else return;
        cx.stop_propagation();
        cx.notify();
      }).overflow_hidden().rounded(10).border_1().border_color(t.border).bg(t.background).text_color(t.foreground)
      .child(bar)
      .when(this.finding, el => el.child(findBar))
      .when(Boolean(status), el => el.child(status))
      .child(body);
  }
}
