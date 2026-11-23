# 前端迁移到 GPUI Kit —— 迁移方案（决策集 / 规格）

> 本文是 wayfinder 地图「前端迁移到 GPUI Kit」的收口交付物：把 10 张决策票
> （可行性、PoC、桥接、组件映射、技术选层、弹窗锚定、架构、验收、依赖分发 CI、收口）
> 的结论汇总成一份**可直接开工**的迁移规格。详细推导见 `.scratch/gpui-migration/issues/`。

**范围**：只迁 `crates/xiaomi-buds5-tray`（GTK3 托盘弹窗）。service 的 D-Bus 契约不变；
GNOME 扩展（shell JS）与 CLI 不动。本文只出方案，落地施工另起 effort。

---

## 1. 目标与终点

把 GTK3 托盘弹窗换成 **GPUI Kit**（Longbridge v0.10，基于 Zed GPUI）渲染的前端：

- 保留现有**手写 StatusNotifierItem**（gio/glib D-Bus）注册顶栏图标，只把弹窗内容换 GPUI。
- 弹窗在 **XWayland（X11 后端）** 下复刻「贴顶栏图标」的定位：**映射后用 raw XID + xcb
  `configure_window` 重定位**（创建时 `window_bounds` 会被 Mutter 忽略，见 §6 P0.5 实测；GNOME
  不实现 `wlr-layer-shell`，见 §2；Wayland 会话下须强制 X11 后端）。
- 功能 **1:1 等价替换**，一个不删（见 §7 验收清单）。
- 最终（P6）删除旧 GTK tray 与 GTK3 依赖，唯一前端为 GPUI。

---

## 2. 关键前提与约束

| 事实 | 来源 | 影响 |
| --- | --- | --- |
| GNOME/Mutter 不实现 `wlr-layer-shell`（本机 Zorin17 / GNOME43 Wayland 无 `zwlr_layer_shell_v1`）；GPUI 建 LayerShell 窗即返 `LayerShellNotSupportedError` | 采票「可行性」+「弹窗锚定」+ PoC | 贴顶栏锚定**只能走 X11 后端（XWayland）** |
| GPUI 后端选择：默认 `WindowingModes::all()`，`select_backend` **优先 Wayland**（`WAYLAND_DISPLAY` 非空即选 Wayland）；`WindowingRequest` 只有 Headless/Windowed，**无「仅 X11」表达** | 源码核实：`gpui-pre-linux` `linux.rs:44` / `display_connection.rs:30-35` / `platform.rs:295-315` | Wayland 会话下须 `Environment=WAYLAND_DISPLAY=` + `DISPLAY=:0` 强制 X11 后端，否则走 Wayland、无法定位 |
| **X11 定位须「映射后重定位」**：创建时 `window_bounds` 绝对坐标被 Mutter 忽略（实测请求 320,160 → 放置 30,30）；map 后 X ConfigureWindow 精确生效（320,160 / 700,400 均精确）；公开 API 无移动原点的 set_bounds（仅内部 `X11WindowStatePtr`） | P0.5 spike 实测（§13） | 实现：`HasWindowHandle` 取 XID → xcb `configure_window` 在 map 后移动（等价旧 GTK show 后 `gtk_window_move`） |
| GPUI 需较新 Rust（≥1.92，本机 default 1.98.1）+ GPU（Vulkan/OpenGL，含 lavapipe 软渲兜底） | PoC | Requirements 更新，见 §8 |
| 本机 PoC 实测只差 `libxkbcommon-x11-dev`（干净 runner 完整清单见 §8） | PoC | CI/install/文档补依赖 |
| GPUI 只 `Application::run` 独占主线程；`App/Entity` 皆 `!Send!Sync` | 票「桥接」 | 桥接必须走 flume + `cx.spawn`，见 §4 |
| gpui-kit 无 Tray/SNI 组件；无 `Stack`；无 `preferred_height`；多段 CSS 阴影/过渡不 1:1 | 票「组件映射」 | SNI 保留旧实现；用状态枚举+measure+近似视觉 |
| `WindowOptions` 无 `always_on_top` / `skip_taskbar` / `mouse_passthrough` | 票「弹窗锚定」 | 已知缺口，见 §10 |

**官方支持路径 = XWayland 可用（Wayland 会话下强制 X11 后端）**。真正前提是 **XWayland**，不是 X11
session——GNOME 移除的是 X11 *session*，XWayland 短期仍在（现有托盘也已靠 `GDK_BACKEND=x11` 跑在
XWayland 上）。无 XWayland 环境的无锚定降级路径见 §12（未来工作，不纳入本次完成 gate）。

---

## 3. crate / 目录结构

新建独立 crate **`crates/xiaomi-buds5-panel-gpui`**，bin 名 **`xiaomi-buds5-panel`**（旧
`crates/xiaomi-buds5-tray` 冻结保留作回滚，P6 删除）。

```
crates/xiaomi-buds5-panel-gpui/
├── Cargo.toml            # gpui-kit 0.10；无 gtk 依赖
└── src/
    ├── main.rs           # 起 1 条 glib 线程 + Application::run(GPUI 主线程)
    ├── ui/               # GPUI 视图：主页 / 设置页 / 组件
    ├── model/            # State 纯数据、parse_state、ServiceCmd 等
    └── dbus/             # SNI（gio）+ service 客户端 + socket 监听，全跑 glib 线程
```

- `State` / `parse_state` / SNI 实现**直接复制**自 tray crate（旧 tray 冻结、P6 净身时删除；
  不抽共享 crate，避免过渡期双份维护成本）。
- workspace `Cargo.toml` 增加该 member（P1 起；P6 删除 tray member）。

---

## 4. 线程与桥接模型

```
GPUI 主线程 (Application::run)          glib 线程 (MainContext + MainLoop)
  ├─ 渲染 UI / Entity                    ├─ 手写 SNI（gio D-Bus，注册顶栏图标）
  ├─ 消费 StateDTO  → cx.notify()        ├─ service 客户端 (gio call_sync / signal_subscribe)
  ├─ 发 SvcCmd / SniCmd                  ├─ unix socket 监听 (show 触发)
  └─ background_executor: 本地系统操作    └─ 3s tick 取 State
```

三条 **flume** 通道（单向解耦，避免 `!Send` 跨线程）：
- `StateDTO → GPUI`：glib 线程拿到/解析出 `State` 后推给 GPUI，UI 在 `cx.spawn` 常驻任务里
  `WeakEntity::update(|this,cx|{ this.state = ..; cx.notify(); })`。
- `SvcCmd → glib`：UI 动作（`SetNoiseMode` / `SetNoiseLevel` / `SetAudioBackend` / `Connect`
  / `Refresh`）→ glib 线程 `call_sync`（沿用现有 gio 客户端，`SetAudioBackend` 超时 75s）。
- `SniCmd → glib`：UI 需要动 SNI 时（toggle）→ glib。

**本地系统操作**（pactl / bluetoothctl 子进程）走 GPUI `background_executor` 线程池，不塞 glib
线程、不在 UI 主线程直调。**退出**：UI 发 `Quit` → glib 线程 join + 清理 socket。

---

## 5. 组件映射（GTK → gpui-kit 0.10）

| GTK 控件 / 行为 | gpui-kit 对应 |
| --- | --- |
| 分段按钮（模式 / 强度 / 后端） | `TabBar.segmented()` |
| 音量 `gtk::Scale` | `Slider` + `SliderState` |
| 各类按钮 | `Button`（`.primary()` 等） |
| 耳机主图（多色 SVG） | `img()`（多色）；功能小图标 `Icon`（单色） |
| 编解码网格（2 列动态） | `grid()` 动态列 |
| 主页 / 设置页切换 | 状态枚举 + `match`（无 `Stack`） |
| 卡片 / CSS `.buds-popover` | `div` + 主题 token（`ThemeColor`） |
| 多段 `box-shadow` | `shadow_sm` 近似 |
| 140ms CSS 过渡 | `Animation`（轻量 / 瞬时） |
| 内容自适应高度（无 preferred_height） | measure 后 `window.resize`（≤720，页切换重测） |
| 失焦关闭 | `observe_window_activation` 失焦即隐藏 |
| 无边框拖拽 | `window_decorations = Client` + `start_window_move` |

**主题**：自定义 buds 主题 —— `ThemeRegistry::watch_dir` + JSON 覆盖 `ThemeColor` token；
浅色复刻 GTK（`#f5f5f7 / #1d1d1f / #007aff`），深色跟随系统。保真底线：结构 / 配色 /
圆角 / 间距 / 字号 / 控件形态 1:1；阴影近似、过渡轻量、高度自适应允许轻微跳变。

---

## 6. 阶段计划与验收 gate

| 阶段 | 内容 | 验收 gate |
| --- | --- | --- |
| **P0 底座** | Rust ≥1.92 pin（`rust-toolchain.toml` **且** CI 改固定版本，见 §8——只 pin 文件不改 CI 等于没 pin）；`libxkbcommon-x11-dev` 及完整构建依赖进 CI/文档；落**能编译的 gpui stub crate**（本阶段即可全量 build）；CI 改造 | 全量 `cargo build`（含 stub crate 的 gpui 依赖）过 + CI 绿 |
| **P0.5 后端 spike（开工闸）✅ 已实测成立** | 已在 XWayland 实测：X11 后端可开无边框窗并渲染（ARGB depth32、scale1.0）；**创建时 `window_bounds` 定位被 Mutter 忽略**（请求 320,160→放置 30,30），须 map 后用 raw XID + xcb `configure_window` 重定位（实测 320,160 / 700,400 均精确）；client-decoration 有 ~20px/边阴影内边距 | 结论：路线 A 地基成立（定位手法=映射后重定位），见 §13 与 `prototype/README.md`；P3 按此实现 |
| **P1 新 crate + 桥接骨架** | 新 crate（ui/model/dbus）；单 glib 线程跑 SNI+client+socket；GPUI 主线程；flume 三通道 | **测试前先 `systemctl --user stop xiaomi-buds5-tray.service`（见下方 P1 冲突注）**；新 bin 启动、SNI 注册成功、点图标弹空窗、能读到 State 并打日志 |
| **P2 UI 功能 1:1** | 按 §7 A–I 清单实现主页（Hero/电量/音量/降噪/动作按钮）+ 设置页（后端/codec） | §7 除「A 组窗口行为」外功能项全过 checklist |
| **P3 弹窗窗口行为** | XWayland（X11 后端）map 后 raw XID+xcb 重定位、测高 resize、失焦/Esc 隐藏、fade 200ms、自绘 header、workarea 夹取、HiDPI 逐屏 | §7 A 组 7 项全过 |
| **P4 打包与切换默认** ✅ | `install.sh --ui=gtk\|gpui`、双 unit、autostart、文档/徽章、CI | `--ui` 两路可选、卸载干净、文档中英对齐、CI 绿（本地验证：release build / fmt / clippy / test / 参数解析 / `bash -n` 全绿；**实机安装 gpui↔gtk 与卸载已跑通，见 §14**） |
| **P5 切默认为 GPUI + 灰度** | 默认 `gpui`（✅ install.sh 已默认 gpui）；旧 GTK 仍保留可一键切回（✅ `--ui=gtk`）；**进 P6 前置：打 tag `pre-gpui-cutover` + 归档旧 release 二进制**（待提交/人工） | §7 全清单在 XWayland 并排全绿 + 一周试用无回归 + tag/旧二进制已归档 |
| **P6 删 GTK 净身** | 见 §9 净身清单 | GTK3 依赖移除后 build/test 仍绿；旧 tray 清空；文档去 transitional |

> **P1 冲突注（单实例靠抢名/抢 socket）**：旧 tray 用 `RequestName("org.kde.StatusNotifierItem-xiaomi-buds5", DO_NOT_QUEUE)`（`crates/xiaomi-buds5-tray/src/main.rs:402-403`），socket 写死 `/tmp/xiaomi-buds5-panel-{uid}.sock`（`main.rs:1332`）——新旧不能同跑，新前端注册 SNI 前必须先停 `xiaomi-buds5-tray.service`（或临时换 SNI 名 / 换 socket 路径）。

---

## 7. 功能对齐与验收清单（引用「功能对齐与验收范围」票）

详细清单见票「功能对齐与验收范围」Answer，摘要分组：**A 弹窗窗口行为**（A1–A7）、**B Hero**、
**C 电量**、**D 音量**、**E 降噪**、**F 动作按钮**、**G 设置页**、**H 全局行为**（tick 3s /
StateChanged / socket show / 无耳机隐藏 / 文案）、**I SNI 链路**（保留旧实现）。

每项格式：「触发 → 期望（UI + service 侧）→ 判定来源（肉眼 / `dbus-monitor`/`gdbus` / 日志）」。
判定 = 混合：checklist 抓行为（主）+ X11 同分辨率新旧并排截图（观感）+ 少量自动化冒烟
（State 解析、组件渲染不 panic、D-Bus 参数断言）。允许近似项仅限：阴影、140ms 过渡、自适应高度。

> 施工交接前，建议把票「功能对齐与验收范围」Answer 里的 **A–I 全表（约 40 项）作为附录粘进本文**（此处仅摘要），否则只读本文的施工方拿不到完整验收点。

---

## 8. 依赖 / 分发 / 文档 / CI 变更

**依赖（迁移期双栈并存，P6 才真正去 GTK3）**
- 过渡期 `Cargo.toml` / CI / `install.sh` 同时保留 GTK3 与 GPUI 两套构建依赖。
- 新增构建期：`libxkbcommon-x11-dev`（PoC 在本机已装一堆 `-dev` 后只差它，**不代表干净 runner 只差一个**；干净容器的完整清单见下方 CI 段）。

**Requirements（README 中英同步）**
- 新增：GPU（Vulkan 1.x / OpenGL，含 lavapipe 软渲兜底）；**XWayland（Wayland 会话下强制 X11 后端）**用于弹窗锚定；构建期 `libxkbcommon-x11-dev` + 完整构建依赖。
- 最小支持环境从「GTK3 + GNOME43」更新为「现代 GPU + Vulkan；XWayland 可用用于贴图标弹窗（须清空 `WAYLAND_DISPLAY` 设 `DISPLAY` 强制 X11 后端）」。

**徽章 / README**
- 徽章 `UI: GTK3` → 迁移期 `UI: GPUI (migrating)`；P6 后 `UI: GPUI`。
- 中英 README 严格对齐：首段「Rust + GTK3」→「Rust（GPUI 迁移中）」；Features「两套前端」→
  「三套/迁移中」；Architecture 表加 `crates/xiaomi-buds5-panel-gpui` 行；安装/使用段注明 `--ui`。

**install.sh / uninstall.sh / systemd（Q4）**
- 新 GPUI UI 用新 unit **`xiaomi-buds5-panel-gpui.service`**（`ExecStart=%h/.local/bin/xiaomi-buds5-panel`）。
- `install.sh --ui=gtk|gpui`（或 `XIAOMI_BUDS5_UI`）选择启用哪个前端 unit；autostart `.desktop` 只装其一。
- `uninstall.sh` 同步加新 bin/unit/desktop。
- ⚠️ 命名冲突：现 `xiaomi-buds5-panel.service` 实跑**守护进程**；P6 重命名见 §9。

**CI**
- `apt-get install` 增**实测完整清单**（勿只写一项）：`libxkbcommon-x11-dev` + `libxkbcommon-dev` + `libwayland-dev` + `libx11-xcb-dev` + `libxcb1-dev` / `libxcb-render0-dev` / `libxcb-shape0-dev` / `libxcb-xfixes0-dev` + `libvulkan-dev`；最终以干净容器 `cargo build --release` 实测为准；`cargo build --release` 编全量。
- **headless 普通 runner 只跑非 UI 单测**；UI 冒烟标 `#[ignore]` 或留需 GPU 的自托管 runner。
- `fmt` / `clippy --all-targets -D warnings` 照旧；沿用 cargo 缓存（801 crate）。

**二进制体积 / 构建时间**
- 基线：debug bin **754MB**、target 4.6G、首编 801 crate **1m41s**（8 核）。
- release + strip（`[profile.release] opt-level=2 lto="thin" strip=true` 已在）**已实测**：stub bin **37MB**、全工作区 release 编 **3m05s**（8 核，含 GPUI+GTK 双栈）；debug bin 764MB、全量 target **14G**。
- 不拆 `no-gui` feature（GPUI 必选、单一构建路径，YAGNI）；体积增大作为迁移代价进发布说明。

---

## 9. 回滚策略与 P6 净身清单

**回滚（每阶段独立可回滚）**
- P0–P1 纯新增不碰旧 tray，回滚 = 不启用新 unit。
- P2–P4 双前端并存，回滚 = `install.sh --ui=gtk`（或 enable `tray.service` / disable gpui unit）。
- P5 切默认后仍保留 tray，可一键切回。
- **P6 删旧 tray 不可就地回滚** → 动手前必打 tag `pre-gpui-cutover`（冻结旧 tray 作基线），
  回滚靠 git revert + 保留上一版 release 二进制。旧 tray 全程冻结不改代码。

**P6 净身清单（一次性，按序执行）**
1. 删 `crates/xiaomi-buds5-tray/`、workspace member、`[[bin]]`。
2. `Cargo.toml` 去 `gtk` / `gdk` / `gdk-pixbuf`（仅 tray 用，删 tray 后自动消失；**保留 `glib`/`gio`——新 crate 的 `dbus/` 仍用 gio**，系统包 `libglib2.0-dev` 留着）。
3. **systemd 源文件改名（严格按序）**：① 守护进程源文件 `systemd/xiaomi-buds5-panel.service`（`ExecStart=…/xiaomi-buds5-service`）改名为 `systemd/xiaomi-buds5-service.service`，已装机上的同名 unit 同步 `stop`/`disable`/改名并 `daemon-reload`；② 再把 GPUI UI 源文件 `systemd/xiaomi-buds5-panel-gpui.service` 改名为 `systemd/xiaomi-buds5-panel.service`（去 `-gpui`），已装 unit 同步。**顺序反了会两个 unit 抢同名。** install/uninstall/docs 同步。
4. **已装用户升级迁移**（`install.sh` 新增路径）：检测到旧 `xiaomi-buds5-panel.service`（跑 `xiaomi-buds5-service`）在运行 → `stop` + `disable` → 装并 `enable` `xiaomi-buds5-service.service` → 再装/启 GPUI UI unit。否则改名的 unit 会把旧守护进程「换头」静默替换/挂掉。
5. `install.sh` 去 tray 安装、`--ui` 简化（只剩 gpui）。
6. `uninstall.sh` 补全：删旧三 unit（panel/tray/adaptive）+ 新 `xiaomi-buds5-service.service` + 新 `xiaomi-buds5-panel.service`(UI)；删旧 bin（service/tray/cli）+ **新 bin `~/.local/bin/xiaomi-buds5-panel`** + **autostart 的 GPUI `.desktop`**。
7. README 中英去「过渡期/双前端」、徽章 `UI: GPUI`、Features 改版。
8. CI 去 `libgtk-3-dev` / `librsvg2-common` —— **开工时先验证 GPUI `img()` 解码 SVG 是否需额外库**，不需要才删 `librsvg2-common`。

---

## 10. 已知限制 / 风险

- **XWayland 依赖**：贴顶栏锚定只在「X11 后端（XWayland）」成立——Wayland 会话下须清空 `WAYLAND_DISPLAY` 设 `DISPLAY` 强制 X11 后端；否则 GPUI 默认走 Wayland 无法定位。**P0.5 已实测成立**（见 §13）。GNOME 移除的是 X11 *session*，XWayland 短期仍在；无 XWayland 时降级见 §12。
- **X11 定位是「映射后重定位」**：创建时 `window_bounds` 的绝对坐标被 Mutter 忽略（实测 320,160→30,30），须 map 后取 XID→xcb `configure_window`。client-decoration 窗口带 ~20px/边阴影内边距（410×300 内容→450×340 X 窗），贴图标坐标/尺寸须按内边距换算，或关闭窗口阴影（P3 定）。
- `WindowOptions` 无 `always_on_top` / `skip_taskbar` / `mouse_passthrough`：不做 keep_above，
  靠失焦关闭兜底；被遮挡再补 X11 `_NET_WM_STATE_ABOVE`。skip_taskbar/passthrough 暂无解。
- 高度自适应靠 measure/resize，允许轻微跳变。
- 体积：GPUI 依赖树大（801 crate）；stub release bin 实测 37MB、debug 764MB。
- 多显示器 HiDPI 需逐屏验收（逻辑坐标按 scale 换算整数像素再给 xcb 重定位）。

---

## 11. 交付与开工指引

- 施工按 §6 的 P0→P6 顺序推进，每阶段过 gate 再进下一阶段。
- 决策来源：`.scratch/gpui-migration/issues/01..10`（每票 Answer 有细节与一手源码行号）。
- PoC 参考工程：`.scratch/gpui-migration/prototype/`（`README.md` 有环境/构建/复现）。
- 本规格只描述「做什么、按什么顺序、怎么验收」；具体代码在施工 effort 里写。

---

## 12. 未来工作（非本次完成 gate）

- **Wayland 无锚定降级路径（B）**：GNOME 50 强制 Wayland 后，GPUI 前端退为无锚定普通窗
  （xdg-toplevel，合成器摆位），或临时退回保 GTK tray。触发条件 = 环境无 XWayland 时。
  写清切换时点与 GTK 兜底。
- **无障碍 / 键盘导航**：GPUI AccessKit 与现 GTK 行为对齐。
- **性能 / 显存**：GPUI GPU 常驻显存（~30–120MB 级）阈值与监控。
- **跨发行版打包**（Flatpak/deb/AUR）——延续既有 out of scope。

---

## 13. 附录：P0.5 后端 spike 实测结论（2026-11-22）

环境：Zorin 17.3 / GNOME43 / Wayland 会话；强制 X11 后端 = `WAYLAND_DISPLAY=`（空）+ `DISPLAY=:0` + `XAUTHORITY=/run/user/1000/.mutter-Xwaylandauth.IG6QY3`（XWayland）。工程 `prototype/gpui-hello/src/bin/spike.rs`（`cargo build --bin spike`）。

| 项 | 结果 |
| --- | --- |
| X11 后端在 XWayland 可用 | ✅ 窗口 map、IsViewable、ARGB depth32、渲染正常、`scale_factor=1.0` |
| 创建时 `window_bounds` 绝对定位 | ❌ 被 Mutter 忽略：请求 (320,160)/410×300 → 实际放置 (30,30)/450×340 |
| map 后 X ConfigureWindow 重定位 | ✅ 精确：move→(320,160) 与 (700,400) 均分毫不差 |
| 应用取自身 XID | ✅ `HasWindowHandle for Window` → `RawWindowHandle::Xcb(x)` → `x.get()` = 0x1a00001，与外部 `xdotool search` 一致 |
| 公开 API 移动原点 | ❌ 无；`set_bounds` 仅在内部 `X11WindowStatePtr`（不可达），公开只有 `resize(size)` |
| 尺寸膨胀 | client-decoration 加 ~20px/边 阴影内边距（+40/+40） |
| 多显示器 | XWayland 暴露 2 屏：1440×2560 +0+0、1920×1080 +1440+892，合计 3360×2560（此机 scale1.0） |

**结论**：路线 A 地基成立。定位手法定为「map 后 raw XID + xcb `configure_window`」（等价旧 GTK `position_popup` 的 show 后 `gtk_window_move`），不依赖 GPUI 未公开的 `set_bounds`。

## 14. 附录：P4 实机安装/切换/卸载验证（2026-11-22）

环境：Zorin 17.3 / GNOME43，用户会话 systemd（`systemctl --user`）。逐项跑 `./install.sh`（默认 gpui）→ `./install.sh --ui=gtk` → `./uninstall.sh`，最后恢复基线。

| 步骤 | 观察 | 结论 |
| --- | --- | --- |
| `./install.sh`（gpui） | daemon `active/enabled`；tray `inactive/disabled`；gpui 单元 `active/enabled`；面板窗口 `IsViewable` 458×768；SNI `org.kde.StatusNotifierItem-xiaomi-buds5` 已注册；autostart 切为 `xiaomi-buds5-panel-gpui.desktop`；State 读到（电量 99/99/–、noise 0/2、codec aptx_adaptive_hq、backend adaptive） | ✅ gpui 路起得来 |
| `./install.sh --ui=gtk` | tray `active/enabled`；gpui 单元 `inactive/disabled`；面板进程退出；tray 进程运行；autostart 切回 `xiaomi-buds5-tray.desktop` | ✅ gtk 路可回滚 |
| `./uninstall.sh` | 0 个 unit 文件；全部 `inactive`；`~/.local/bin` 无 xiaomi/buds/panel bin；无 autostart；无 unit 文件残留；无进程 | ✅ 卸载干净 |

**注**：
- GPUI 面板作为 systemd 用户服务启动时，用户管理器环境已含 `DISPLAY=:0`/`XAUTHORITY`/`WAYLAND_DISPLAY`，`main()` 强制 X11 后端后窗口正常 map。
- 安装前 `~/.local/bin/xiaomi-buds5-panel` 曾是指向已消失 `bin/` 的悬空软链；`install.sh` 已如 tray/cli 一样先 `rm -f` 再 install，避免 install 跟随悬空软链失败。
- 验证后机器已恢复基线（daemon + tray active/enabled，adaptive active，gpui 单元 disabled 未启用）。要切到 GPUI 默认：`./install.sh`；回退：`./install.sh --ui=gtk`。
