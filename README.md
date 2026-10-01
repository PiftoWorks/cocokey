# CoCoKey

A layer HUD for your QMK / VIA / Vial keyboard.

An always-on-top overlay that shows which layer your keyboard is on, what
every key does on that layer, and how to get to the other layers.

It reads the keymap straight from the keyboard, so there is nothing to
configure: plug in a Vial or VIA keyboard and the HUD draws it.

- **Live layer tracking** — follows `MO`, `TO`, `TG`, `OSL`, `TT`, `LT`, `DF`
  and tap dances by watching the switch matrix, or uses the exact state if the
  firmware reports it ([how](docs/firmware-layer-report.md)).
- **Real key positions** — Vial keyboards are drawn from the layout stored in
  their firmware. VIA keyboards (including ones set up with Remap) use VIA's
  public definition catalog, or a definition file you drop on the window.
- **Plain keyboards too** — when you type on an ordinary keyboard the HUD
  switches to a standard US or JIS layout and shows Shift and held keys.
- **Macros** — shows what every macro does, on the keys and in a full list.
- **How to switch** — lists every key on the current layer that changes the
  layer, including each tap dance gesture (tap / hold / double tap).
- **Pressed keys** light up; keys that fall through to a lower layer are shown
  dimmed.
- **Mouse layers** — keyboards that jump to a mouse layer when their built-in
  trackball or trackpoint moves are followed too: Svalboard, Keyball, cocot,
  Charybdis / Dilemma, torabo-tsuki and mtk64erp. Where the keyboard ships with
  the feature off, switch it on for that keyboard in the menu.
- Resize by dragging any edge or corner, move by dragging the top bar,
  adjustable opacity, click-through mode, per-keyboard layer names.

## Requirements

- Windows 10 / 11
- For layer tracking: a keyboard running [Vial](https://get.vial.today/) or
  VIA firmware. VIA keyboards need a layout definition: it is downloaded from
  VIA's catalog by USB id, or you drop the keyboard's VIA JSON file (as
  distributed for VIA / Remap) onto the window. Firmware older than QMK 0.19
  is supported; its keycodes are translated.
- The keyboard must be **unlocked** in Vial for layer tracking by key presses
  (Vial blocks reading the switch matrix while locked). Keyboards built with
  `VIAL_INSECURE`, like the Svalboard, are always unlocked.

## Usage

Start CoCoKey and connect the keyboard; it is picked up automatically.

| Action | How |
|---|---|
| Move | drag the top bar |
| Resize | drag any edge or corner |
| Preview another layer | click its number in the top bar (click again or press Esc to go back) |
| Rename a layer | double-click the layer name |
| Macro list | `M` button |
| Keep on top | pin button |
| Add a VIA definition | drop the `.json` file on the window |
| Settings | `⋯` button |
| Click-through, pause, reload, quit | tray icon menu |

Vial and CoCoKey share the same USB interface, and every reply on it reaches
every program that has it open. CoCoKey therefore stops talking to the
keyboard while the Vial or VIA desktop app is running and reloads the keymap
when it closes. If you use a web configurator (Vial web, VIA web, Remap), choose **Pause**
from the tray menu before editing and **Reload keymap** afterwards; browsers
cannot be detected.

## Building

```sh
npm install
npm run tauri dev      # run in development mode
npm run tauri build    # create the installer
npm test               # frontend tests
cd src-tauri && cargo test
```

Requires Node.js 20+, Rust 1.85+ and the
[Tauri prerequisites](https://tauri.app/start/prerequisites/).

## How it works

CoCoKey talks to the keyboard over the VIA raw HID interface
(usage page `0xFF60`):

1. reads the layer count, the keymap, the tap dance table and the tapping term;
2. downloads the keyboard definition that Vial stores in the firmware
   (matrix size and physical layout);
3. polls the switch matrix about 120 times a second and replays the layer
   keycodes with the same rules QMK/Vial use.

Nothing is written to the keyboard. To tell which keyboard you typed on last,
CoCoKey listens to raw keyboard input; it only keeps the set of keys that are
held right now and records nothing.

## Limitations

- Layer tracking by key presses is a replay. Combos, key overrides and custom
  firmware code that change layers are not seen. Use the firmware extension for
  exact tracking.
- Labels assume a US keyboard layout in the operating system.

## License

CoCoKey is released under the [GNU General Public License v3.0](LICENSE).
The firmware add-on in [`firmware/`](firmware/) is released under the
[GNU General Public License v2.0 or later](firmware/LICENSE), the same terms as QMK,
so it can be built into any QMK, VIA or Vial firmware.

---

## 日本語

CoCoKey（ココキー）は、キーボードがいまどのレイヤーにいるか、そのレイヤーで各キーが何をするか、
ほかのレイヤーへの切り替え方を、常に手前に表示するオーバーレイです。

- Vial 対応キーボードをつなぐだけで、キー配列と物理配置を本体から読み込みます。
- VIA 対応キーボード（Remap で設定するものを含む）は、VIA の公開カタログから配置データを
  自動で取得します。カタログにないものは、定義ファイル（VIA 形式の JSON）を
  ウィンドウにドラッグ＆ドロップしてください。古い QMK のファームウェアにも対応しています。
- 普通のキーボードで打つと、標準配列（US / JIS）の表示に切り替わります。
- キーの押下からレイヤーの切り替え（MO / TO / TG / OSL / TT / LT / DF /
  タップダンス）を追跡します。ファームウェアに[小さな追加](docs/firmware-layer-report.md)
  をすると、キーボードから正確なレイヤーを受け取れます。
- Svalboard は指ごとのクラスタ配置で表示します。
- トラックボールを動かすとマウスレイヤーに切り替わるキーボード（Svalboard・Keyball・
  cocot・Charybdis / Dilemma・torabo-tsuki・mtk64erp）は、その切り替わりにも追従します。
  キーボード側の初期設定でこの機能がオフの機種は、メニューでオンにしてください。
- 上部をドラッグで移動、縁や角のドラッグで拡大縮小。不透明度、クリック透過、
  レイヤー名の変更に対応しています。
- Vial（デスクトップ版）の起動中は通信を止め、閉じるとキー配列を読み直します。
  ブラウザで動く設定ツール（Vial / VIA / Remap）で編集するときは、タスクトレイのメニューから「一時停止」を選び、
  編集が終わったら「キー配列を読み直す」を選んでください。

## ライセンス

CoCoKey は [GNU General Public License v3.0](LICENSE) で公開しています。
[`firmware/`](firmware/) のファームウェア用の追加部品は、QMK と同じ
[GNU General Public License v2.0 以降](firmware/LICENSE) です。QMK / VIA / Vial の
ファームウェアにそのまま組み込めます。
