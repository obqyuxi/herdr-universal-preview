// Connect your phone: shows the DD-up HarmonyOS app's pairing QR code
// (`ddup-bridge pair --json`, one-time, 10 minutes), says when the bridge
// isn't running, and closes once a new phone has paired. Opened from the
// phone button beside Settings (workspace-chrome-plus). D27, docs/harmony/.

import { View, div } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { exec, home, theme, props, closeOverlay } from "herdr";

const bridge = () => `${home()}/Library/Application Support/DD-up/bin/ddup-bridge`;
const CELL = 5; // px per QR module
const QUIET = 4; // modules of white border, as scanners expect

async function run(args) {
  try {
    return await exec(bridge(), args);
  } catch (e) {
    return { code: -1, stdout: "", stderr: String(e) };
  }
}

export default class PhonePair extends View {
  init(_props, cx) {
    this.data = null;
    this.error = null;
    this.paired = null;
    this.devices = null;
    this.now = Date.now();
    this.handle = cx.focus_handle();
    this.handle.focus();
    this.generate(cx);
    cx.timer.every(2000, (cx) => this.poll(cx));
    cx.timer.every(15000, (cx) => {
      this.now = Date.now();
      cx.notify();
    });
  }

  generate(cx) {
    this.data = null;
    this.error = null;
    this.paired = null;
    cx.notify();
    cx.spawn(async (cx) => {
      const r = await run(["pair", "--json"]);
      if (r.code === 0) {
        try {
          this.data = JSON.parse(r.stdout);
          this.devices = this.data.devices;
        } catch {
          this.error = "ddup-bridge 的输出无法解析";
        }
      } else {
        this.error = r.code === -1 || /No such file|not found|ENOENT/i.test(r.stderr)
          ? "还没安装桥接服务：在仓库里运行 scripts/bridge-service install"
          : (r.stderr || "生成二维码失败").trim().split("\n").pop();
      }
      this.now = Date.now();
      cx.notify();
    });
  }

  // A new device in `devices --json` means the code was just used.
  poll(cx) {
    if (!this.data || this.paired) return;
    cx.spawn(async (cx) => {
      const r = await run(["devices", "--json"]);
      if (r.code !== 0) return;
      let list = [];
      try {
        list = JSON.parse(r.stdout).devices ?? [];
      } catch {
        return;
      }
      if (this.devices !== null && list.length > this.devices) {
        const newest = list.reduce((a, b) => (b.created_ms > a.created_ms ? b : a));
        this.paired = newest.name;
        cx.notify();
        cx.timer.after(1800, () => this.close());
      }
    });
  }

  close() {
    closeOverlay(props().overlay);
  }

  key(e, cx) {
    if (e.key === "escape") {
      this.close();
      cx.stop_propagation();
    }
  }

  // Each row as runs of the same color: far fewer elements than one per module.
  qr(rows) {
    const n = rows.length + QUIET * 2;
    const white = "#ffffff";
    const black = "#000000";
    const line = (bits) => {
      const runs = [];
      let at = 0;
      while (at < bits.length) {
        let end = at;
        while (end < bits.length && bits[end] === bits[at]) end++;
        runs.push(div().h(CELL).w((end - at) * CELL).flex_none().bg(bits[at] === "1" ? black : white));
        at = end;
      }
      return h_flex().h(CELL).flex_none().children(runs);
    };
    const pad = "0".repeat(QUIET);
    const blank = "0".repeat(n);
    const all = [...Array(QUIET).fill(blank), ...rows.map((r) => pad + r + pad), ...Array(QUIET).fill(blank)];
    return v_flex().flex_none().w(n * CELL).h(n * CELL).bg(white).rounded(6).children(all.map(line));
  }

  render() {
    const t = theme();
    const d = this.data;
    const minutes = d ? Math.max(0, Math.ceil((d.expires_ms - this.now) / 60000)) : 0;
    const expired = d && minutes === 0;
    const body = this.error
      ? div().py(20).text_size(13).text_color(t.blocked).child(this.error)
      : this.paired
        ? v_flex().py(40).items_center().gap(8)
          .child(div().text_size(28).text_color(t.accent).child("✓"))
          .child(div().text_size(14).child(`已连接：${this.paired}`))
        : !d
          ? div().py(40).text_size(13).text_color(t.muted).child("正在生成二维码…")
          : v_flex().items_center().gap(12).pt(6)
            .child(expired ? div().py(60).text_size(13).text_color(t.muted).child("二维码已过期") : this.qr(d.qr))
            .child(div().text_size(12.5).text_color(t.muted).child("打开手机上的「码上」，点「扫码配对」"))
            .when(!d.serving, (el) => el.child(div().text_size(12).text_color(t.blocked)
              .child("桥接服务没在运行，手机连不上：运行 scripts/bridge-service install")))
            .child(div().text_size(11.5).text_color(t.dim)
              .child(`${d.name} · ${d.hosts.join("、")}:${d.port} · ${expired ? "已失效" : `${minutes} 分钟内有效，只能用一次`}`));
    return v_flex()
      .id("phone-pair")
      .track_focus(this.handle)
      .on_key_down((e, cx) => this.key(e, cx))
      .w_full()
      .p(18)
      .bg(t.popover)
      .text_color(t.foreground)
      .child(h_flex().items_center()
        .child(div().flex_1().text_size(15).font_semibold().child("连接手机"))
        .child(div().text_size(11).text_color(t.dim).child("Esc")))
      .child(body)
      .when(!this.paired, (el) => el.child(h_flex().pt(14).gap(8).justify_end()
        .child(div().id("pp-new").h(30).px(12).flex().items_center().rounded(7).border_1().border_color(t.border)
          .text_size(12.5).cursor_pointer().hover((s) => s.bg(t.hover))
          .on_click((_e, cx) => this.generate(cx)).child("换一个"))
        .child(div().id("pp-close").h(30).px(12).flex().items_center().rounded(7).bg(t.solid).text_color(t.solid_foreground)
          .text_size(12.5).cursor_pointer().on_click(() => this.close()).child("完成"))));
  }
}
