// M1 prototype: hosts one native view (web / pdf / media / quicklook) and drives it
// with whitelisted actions, so the host can be checked in a real window.
// Works on stock Herdr too: it then says native views are unavailable.
// The element is a host component: herdr.nativeView.new(id, props), like Editor.new().
import { View, div } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import * as herdr from "herdr";

const SAMPLES = [
  ["站点(交互)", { kind: "web", file: "site/index.html", trust: "interactive" }],
  ["站点(静态)", { kind: "web", file: "site/index.html", trust: "static" }],
  ["example.com", { kind: "web", url: "https://example.com" }],
  ["PDF", { kind: "pdf", file: "sample-3pages.pdf" }],
  ["MP4", { kind: "media", file: "sample-h264-aac.mp4" }],
  ["MOV", { kind: "media", file: "sample-h264-aac.mov" }],
  ["MP3", { kind: "media", file: "sample-speech.mp3" }],
  ["WAV", { kind: "media", file: "sample-speech-pcm.wav" }],
  ["M4A", { kind: "media", file: "sample-speech-aac.m4a" }],
  ["DOCX", { kind: "quicklook", file: "sample-3pages.docx" }],
  ["XLSX", { kind: "quicklook", file: "sample-3sheets.xlsx" }],
  ["PPTX", { kind: "quicklook", file: "sample-4slides.pptx" }],
  ["坏PDF", { kind: "pdf", file: "corrupt-truncated.pdf" }],
];

const ACTIONS = {
  web: [["后退", "browser.back"], ["前进", "browser.forward"], ["刷新", "browser.reload"], ["停止", "browser.stop"]],
  pdf: [["上一页", "pdf.previous"], ["下一页", "pdf.next"], ["第2页", "pdf.goToPage", { page: 2 }], ["放大", "pdf.zoomIn"], ["缩小", "pdf.zoomOut"], ["适应宽度", "pdf.fitWidth"]],
  media: [["播放", "media.play"], ["暂停", "media.pause"], ["跳到4秒", "media.seek", { time: 4 }], ["静音", "media.setMuted", { muted: true }], ["取消静音", "media.setMuted", { muted: false }]],
  quicklook: [],
};

export default class NativeProbe extends View {
  init(_props, cx) {
    // Layout props come from herdr.props() (as in the shipped pane/peek mods).
    const props = herdr.props();
    this.slot = props?.slot ?? "a";
    this.samples = props?.samples ?? "";
    this.pick = props?.start ?? 0;
    this.visible = true;
    this.open = true;
    this.generation = 0;
    this.log = [];
    this.result = "";
    this.available = typeof herdr.nativeView?.new === "function";
    this.caps = this.available ? herdr.nativeViewCapabilities() : null;
    if (this.available) {
      cx.spawn(async (cx) => {
        for (;;) {
          const e = JSON.parse(await herdr.nextNativeViewEvent());
          if (e.id !== this.id()) continue;
          // Late events from an earlier load are shown, but marked and not applied.
          const stale = e.generation < this.generation;
          if (!stale && e.event !== "view.disposed") this.generation = e.generation;
          this.log = [`${stale ? "(旧)" : ""}g${e.generation} ${e.event}${e.data?.title ? " · " + e.data.title : ""}${e.data?.message ? " · " + e.data.message : ""}${e.data?.page ? ` · 第${e.data.page}/${e.data.pageCount}页` : ""}`, ...this.log].slice(0, 6);
          cx.notify();
        }
      });
    }
  }

  id() { return `view-${this.slot}`; }

  props() {
    const s = SAMPLES[this.pick][1];
    return s.url ? { kind: s.kind, url: s.url } : { kind: s.kind, path: `${this.samples}/${s.file}`, ...(s.trust ? { trust: s.trust } : {}) };
  }

  call(action, args, cx) {
    try {
      const r = herdr.nativeViewCall(this.id(), action, args ?? {});
      this.result = `${action} → ${JSON.stringify(r)?.slice(0, 160) ?? "ok"}`;
    } catch (e) {
      this.result = `${action} ✗ ${e?.message ?? e}`;
    }
    cx.notify();
  }

  button(t, id, label, onClick, on = false) {
    return div().id(id).px(8).h(24).flex_none().items_center().flex().rounded(6).cursor_pointer().text_size(12)
      .bg(on ? t.accent : t.raised).text_color(on ? t.accent_foreground ?? t.background : t.foreground)
      .hover(s => s.bg(t.hover)).on_click((_e, cx) => onClick(cx)).child(label);
  }

  render(cx) {
    const t = herdr.theme();
    const kind = SAMPLES[this.pick][1].kind;
    const head = h_flex().flex_wrap().gap(4).p(6).flex_none().border_b(1).border_color(t.border)
      .children(SAMPLES.map(([label], i) => this.button(t, `s-${this.slot}-${i}`, label, (cx) => { this.pick = i; this.open = true; this.log = []; cx.notify(); }, i === this.pick && this.open)));
    const tools = h_flex().flex_wrap().gap(4).px(6).py(4).flex_none().border_b(1).border_color(t.border)
      .children((ACTIONS[kind] ?? []).map(([label, action, args], i) => this.button(t, `a-${this.slot}-${i}`, label, (cx) => this.call(action, args, cx))))
      .child(this.button(t, `st-${this.slot}`, "状态", (cx) => this.call("state", {}, cx)))
      .child(this.button(t, `vis-${this.slot}`, this.visible ? "隐藏视图" : "显示视图", (cx) => { this.visible = !this.visible; this.call("setVisible", { visible: this.visible }, cx); }))
      .child(this.button(t, `blur-${this.slot}`, "焦点还给Herdr", (cx) => this.call("blur", {}, cx)))
      .child(this.button(t, `close-${this.slot}`, this.open ? "关闭视图" : "重新打开", (cx) => { this.open = !this.open; this.generation = 0; cx.notify(); }));
    const status = v_flex().px(8).py(4).flex_none().text_size(11).text_color(t.dim).border_b(1).border_color(t.border)
      .child(div().truncate().child(`${this.id()} · ${kind} · ${this.available ? this.caps?.interface : "当前 Herdr 未提供原生预览（nativeView 不存在）"} · g${this.generation}`))
      .child(div().truncate().child(this.result || " "))
      .children(this.log.slice(0, 3).map((l, i) => div().id(`log-${this.slot}-${i}`).truncate().child(l)));
    const placeholder = v_flex().size_full().items_center().justify_center().text_color(t.dim).text_size(12)
      .child(this.open ? "原生视图区域：加载中，或被 Herdr 菜单/对话框暂时遮挡（视图状态保留）" : "视图已关闭（原生视图已销毁）");
    const body = div().id(`body-${this.slot}`).flex_1().min_h(0).relative().bg(t.background)
      // The placeholder is the element's child: drawn underneath the native view.
      .child(this.available && this.open ? herdr.nativeView.new(this.id(), this.props()).child(placeholder) : placeholder);
    return v_flex().id(`probe-${this.slot}`).size_full().min_h(0).overflow_hidden().bg(t.surface ?? t.background).text_color(t.foreground)
      .child(head).child(tools).child(status).child(body);
  }
}
