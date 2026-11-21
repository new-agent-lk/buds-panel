# 可选的 aptX Adaptive 音频栈

Buds Panel 的主程序可在 **SBC / AAC / aptX Classic** 之间自由切换，这由系统自带的
PipeWire / PulseAudio 直接完成，无需任何额外组件。

要让面板额外提供 **aptX Adaptive** 系列（Adaptive / Adaptive HQ / Adaptive LL），需要一套
**仓库之外**的独立音频栈。本文说明它是什么、为什么不在本仓库里、怎样自行准备，以及缺失时
Buds Panel 如何自动降级。

> 一句话：**不开源分发，按需自备。** 面板检测不到就静默回退到系统音频服务器，不会报错。

---

## 1. 它是什么

aptX Adaptive 是由高通授权的编解码器，标准 PipeWire / PulseAudio 只带 SBC / AAC / aptX Classic。
本项目配套的是一套隔离的实验栈：

- 一套独立的 **PipeWire 1.7** + **WirePlumber**（构建产物放在 `runtime/pipewire-build/`），
- 打了补丁的 BlueZ SPA 插件与一个 **aptX Adaptive 编码器** `libspa-codec-bluez5-aptx.so`，
- 由 `audio-adaptive/` 下的脚本按需拉起 / 关闭（`xiaomi-buds5-adaptive.service` → `run.sh`）。

该编码器的源码是一个独立的实验项目
[`AKS-Logic/aptx-adaptive-linux`](https://github.com/AKS-Logic/aptx-adaptive-linux)（MIT）。
Buds Panel 只负责“编排”它，**不包含、也不链接它的源码**。

## 2. 为什么不在本仓库

- **专利**：aptX 系列受高通专利约束。源码外置、v0.1 只发主程序源码、不随附任何 aptX 二进制，
  是刻意的合规隔离（详见上游项目的 `LEGAL.md` 与本仓库的 `NOTICE`）。
- **构建难度**：它需要一套打过补丁的 PipeWire / BlueZ / bluetoothd 与固定版本的 Rust 工具链，
  对本项目的大多数用户是多余负担。默认走系统音频服务器已经能覆盖 SBC / AAC / aptX Classic。

## 3. 面板如何使用它

在面板「声音设置」页选 **高质量模式**时，Buds Panel 会 `systemctl --user start
xiaomi-buds5-adaptive.service`，由 `audio-adaptive/start.sh` 拉起隔离音频栈，然后把耳机
A2DP endpoint 交给它，并优先选 `aptx_adaptive_hq` profile。

`audio-adaptive/` 脚本的关键行为：

- **只是编排，不含数据**：`start.sh` 从 `runtime/pipewire-build/`（PipeWire 1.7 构建树）与
  编码器所在目录取出运行所需文件，放到 `~/.local/share/xiaomi-buds5-panel/adaptive-runtime/`。
- **单一音频服务器**：启动前会 `stop` + `mask` 系统的 pulseaudio / pipewire 单元，避免两个服务器
  抢同一块 ALSA 设备；`stop.sh` 退出时恢复原来那一个后端。
- **健康检查与回滚**：若拉起后耳机 card 或 A2DP sink 没出现，`start.sh` 自动调用 `stop.sh`
  恢复原后端并 `exit 1`——不会留下“服务 active 但没声”的假象。

相关环境变量（`audio-adaptive/env.sh` + `start.sh` 中设置）：

| 变量 | 作用 |
| --- | --- |
| `BUDS_MAC` | 耳机 MAC；`XIAOMI_BUDS5_MAC` 可覆盖，否则自动从 `bluetoothctl paired-devices` 发现。 |
| `XIAOMI_BUDS5_APTX_RUNTIME` | 可选栈根目录（含 `pipewire-build/` 与 `payload/spa`）。优先于默认位置。 |
| `APTX_STACK` | `env.sh` 解析出的栈根：优先 `XIAOMI_BUDS5_APTX_RUNTIME`，其次 `~/.local/share/xiaomi-buds5-panel/aptx-stack`，再次仓库内 `runtime/`；都无则空。 |
| `BUILD` | 栈内的 PipeWire 1.7 构建树（`$APTX_STACK/pipewire-build`）。 |
| `APTX_PAYLOAD` | 插件 `.so` 与 wireplumber 配置来源（`$APTX_STACK/payload` 或 `experimental/run`）。 |
| `RUNTIME` / `LOG` / `STATE` | 运行时目录 / 日志 / 状态，均在 `~/.local/share/xiaomi-buds5-panel/`。 |
| `APTX_ADAPTIVE_DEFAULT_MODE` | 默认档位，脚本设为 `hq`。 |
| `APTX_ADAPTIVE_THREE_PROFILES=1` | 暴露 Adaptive / HQ / LL 三档。 |
| `APTX_ADAPTIVE_SOURCE_TYPE_OVERRIDES=${BUDS_MAC}=2` | 强制该耳机用 host-timed（source-type 2）。**关键。** |
| `APTX_ADAPTIVE_LOG_SOURCE_TYPE=1` | 在日志里记录选中的 source-type，便于排查。 |

> **关于 `APTX_ADAPTIVE_ST0`（重要）**：旧的默认值 `ST0=1` 会选 source-type 0（source-timed），
> 需要耳机侧提供蓝牙链路时钟；主机编码器拿不到该时钟时，**sink 收流却渲染静音**——这正是
> Xiaomi Buds 5 在 `ST0=1` 下“有流无声”的原因（见上游 `docs/adr/0001-architecture.md`）。
> 因此本项目的 `start.sh` 默认 **不设** `ST0`，改用 source-type 2（host-timed）。若你的耳机
> 恰好相反（需要 0x00，例如某些 Bose 型号），才手动设 `APTX_ADAPTIVE_ST0=1` 并去掉 override。

## 4. 如何获取 / 自备并构建

栈不随源码仓库分发。获取方式二选一：

**方式 A：下载预编译栈（省事）**

```sh
scripts/fetch-aptx-stack.sh --url <Release 资产 tarball 的 URL>
# 或从已有的栈目录复制：
scripts/fetch-aptx-stack.sh --from <已有栈目录>
```

脚本把栈装到 `~/.local/share/xiaomi-buds5-panel/aptx-stack/`（或 `--target` 指定处），并校验
`pipewire-build/daemon/pipewire` 与 `payload/spa` 是否齐全，最后打印 `export XIAOMI_BUDS5_APTX_RUNTIME=...`。
随后在面板「声音设置」选音频后端 adaptive / 协议 aptX Adaptive 即可。

**方式 B：自行构建（进阶）**

> 以下为上游项目的通用流程，具体命令以其 `README.md` 为准。整套构建较重，且需要与你的
> 发行版内核/BlueZ 匹配。

1. 获取上游源码：`git clone https://github.com/AKS-Logic/aptx-adaptive-linux`。
2. 按上游 `README.md` 准备：打过补丁的 **PipeWire**（补丁 `patches/pipewire/0001-bluez5-aptx-adaptive.patch`）、
   打过补丁的 **bluetoothd**，以及其要求的 **Rust 工具链**（项目通过 `mise` 固定版本）。
3. 构建插件：`mise run install-plugin`（产出一个 `libspa-codec-bluez5-aptx.so`）。
4. 把构建出的 PipeWire 1.7 树放到 `runtime/pipewire-build/`（或 `payload/`/`experimental/run` 布局），
   或直接用 `XIAOMI_BUDS5_APTX_RUNTIME` 指向构建产物目录。
5. 启动高质量模式：面板切「高质量模式」，或手动 `systemctl --user start xiaomi-buds5-adaptive.service`。

> `runtime/`、`experimental-aptx-adaptive/`、`aptx-adaptive-linux/` 均被 `.gitignore` 排除在仓库之外；
> 一键取栈脚本见 `scripts/fetch-aptx-stack.sh`。

## 5. 缺失时的自动降级

完全没有这套栈、或它启动失败时，Buds Panel **不会因此失真**：

- 服务只在“隔离栈正在运行”（`adaptive.pid` 存在）时才把 `PULSE_SERVER` 指向私有 socket，
  否则一律用系统 `pactl`。
- 高质量模式启动失败时，`start.sh` 自行回滚并 `exit 1`，服务把错误写进 `LastError`，面板显示在
  状态栏；主界面电量 / 降噪 / 音量不受影响。
- 编码列表里**仍会列出** aptX Adaptive HQ / 动态 / LL 三项，但标注「（需安装可选栈）」，提示你
  这项功能存在、需取栈后才能用；SBC / AAC / aptX Classic 照常可用。

## 6. 故障排查

| 现象 | 排查 |
| --- | --- |
| 切高质量模式失败 / 无“有流无声” | 看 `~/.local/share/xiaomi-buds5-panel/adaptive-logs/{pipewire,wireplumber,pipewire-pulse}.log`；确认 `source-type 2`（`APTX_ADAPTIVE_LOG_SOURCE_TYPE=1`）。 |
| “card did not appear” / sink 没出现 | `start.sh` 已自动回滚；多为耳机未连、被手机占用，或 `BUILD`/编码器 `.so` 路径不对。 |
| 有流但耳机没声 | 极可能是误开了 `ST0`（source-type 0）——去掉 `APTX_ADAPTIVE_ST0`，保留 `SOURCE_TYPE_OVERRIDES=…=2`。 |
| 桌面应用出声走了别处 | 隔离栈抢占了标准 pulse socket；确认 `PULSE_SERVER=unix:$RUNTIME/pulse/native` 与默认 sink 指向 `bluez_output.*`。 |
| 切回兼容模式后系统没声 | `stop.sh` 恢复原后端失败；手动 `systemctl --user restart pulseaudio.service`（或 pipewire 相关单元）。 |

---

本组件为独立、非官方实验实现，与高通、小米均无隶属关系。aptX 是高通的商标与专利技术；
使用前请自行确认你所在司法辖区的授权情况。
