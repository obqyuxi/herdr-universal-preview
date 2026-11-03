// browser (herdr-universal-preview): a web page inside Herdr's side panel.
// Type an address and press Enter. Logins are kept (this app's own WebKit
// store, separate from Safari and Chrome). Needs Herdr Preview Dev.
import { View, div } from "gpui-kit";
import { v_flex, h_flex, InputState, Input } from "gpui-base";
import * as herdr from "herdr";

const ID = "browser";
const hasNative = () => typeof herdr.nativeView?.new === "function";
// Chromium when the host has it (Herdr Preview Dev with CEF), else WebKit.
const engine = () => (herdr.nativeViewCapabilities?.().kinds ?? []).some(k => k.kind === "chromium") ? "chromium" : "web";

export default class Browser extends View {
  init(_props, cx) {
    // No default site: the first time it waits for an address; later it
    // reopens the last page.
    this.start = localStorage.getItem("browser.url") || null;
    this.url = this.start;
    this.title = "";
    this.error = null;
    this.loading = false;
    this.canBack = false;
    this.canForward = false;
    this.input = InputState.new({ placeholder: "输入网址，回车打开（例如 github.com 或 localhost:3000）", value: this.start ?? "" });
    this.input.on("submit", (_e, cx) => this.go(this.input.value(), cx));
    if (!hasNative()) return;
    this.engine = engine();
    cx.spawn(async (cx) => {
      for (;;) {
        const e = JSON.parse(await herdr.nextNativeViewEvent());
        if (e.id !== ID) continue;
        const d = e.data ?? {};
        if (e.event === "load.start") { this.loading = true; this.error = null; }
        if (e.event === "load.commit" || e.event === "load.finish") {
          this.loading = e.event !== "load.finish";
          if (d.url) { this.url = d.url; this.input.set_value(d.url); localStorage.setItem("browser.url", d.url); }
        }
        if (e.event === "load.error") { this.loading = false; this.error = d.message ?? "打开失败"; }
        if (e.event === "web.processTerminated") this.error = "页面进程意外退出，请点刷新";
        if ("title" in d) this.title = d.title ?? "";
        if ("canGoBack" in d) { this.canBack = d.canGoBack; this.canForward = d.canGoForward; }
        cx.notify();
      }
    });
  }

  call(action, args) {
    try { return herdr.nativeViewCall(ID, action, args ?? {}); }
    catch (e) { this.error = String(e?.message ?? e); return null; }
  }

  go(text, cx) {
    this.error = null;
    if (!text.trim()) return;
    // The first address creates the view; later ones navigate inside it.
    if (!this.start) { this.start = text.trim(); cx.notify(); return; }
    this.call("browser.navigate", { url: text.trim() });
    this.call("focus");
    cx.notify();
  }

  button(t, id, label, enabled, onClick) {
    return div().id(id).w(28).h(26).flex_none().flex().items_center().justify_center().rounded(6).text_size(14)
      .text_color(enabled ? t.foreground : t.dim)
      .when(enabled, el => el.cursor_pointer().hover(s => s.bg(t.hover)).on_click((_e, cx) => { onClick(); cx.notify(); }))
      .child(label);
  }

  render(cx) {
    const t = herdr.theme();
    if (!hasNative()) {
      return v_flex().size_full().p(16).bg(t.background).text_color(t.dim).text_size(13)
        .child("当前 Herdr 版本不能在窗口内显示网页（需要 Herdr Preview Dev）。");
    }
    const bar = h_flex().h(40).flex_none().px(6).gap(2).items_center().border_b(1).border_color(t.border)
      .child(this.button(t, "b-back", "←", this.canBack, () => this.call("browser.back")))
      .child(this.button(t, "b-fwd", "→", this.canForward, () => this.call("browser.forward")))
      .child(this.button(t, "b-reload", this.loading ? "✕" : "↻", true, () => this.call(this.loading ? "browser.stop" : "browser.reload")))
      .child(div().flex_1().min_w(0).child(Input.new(this.input)));
    // Only a problem gets a line; the page title stays on the page.
    const status = this.error
      ? div().px(10).py(3).flex_none().text_size(11).truncate().text_color(t.blocked).child(`打不开：${this.error}`)
      : null;
    return v_flex().id("browser").size_full().min_h(0).overflow_hidden().rounded(10).border_1().border_color(t.border).bg(t.background).text_color(t.foreground)
      .child(bar)
      .when(Boolean(status), el => el.child(status))
      .child(div().flex_1().min_h(0).relative()
        .when(!this.start, el => el.child(div().p(16).text_color(t.dim).text_size(13).child("在上面输入网址，回车打开")))
        .when(Boolean(this.start), el => el.child(herdr.nativeView.new(ID, { kind: this.engine, url: this.start, persist: true }))));
  }
}
