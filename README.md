# Xiaomi Buds 5 Panel

GNOME 43 顶栏快捷设置控件，通过 Xiaomi Buds 5 的 Bluetooth Classic RFCOMM 控制通道读取左右耳/充电盒电量、切换降噪模式，并通过系统音频后端控制耳机音量。

## 已安装内容

- `~/.local/share/gnome-shell/extensions/xiaomi-buds5-panel@kang`
- 用户服务 `xiaomi-buds5-panel.service`（Rust 后端 `xiaomi-buds5-service`）与 `xiaomi-buds5-tray.service`（Rust 托盘 `xiaomi-buds5-tray`）
- 命令行工具 `~/.local/bin/xiaomi-buds5`（Rust，薄 D-Bus 客户端）
- 源码与安装脚本：仓库根目录（后端与托盘均为 Rust，`cargo build --release` 构建；`install.sh` 会自动完成）

## 使用

1. 确认耳机已经和电脑配对并连接。
2. 注销并重新登录一次 GNOME。GNOME 43 在登录时扫描扩展目录；新扩展安装后，当前会话不会自动发现。
3. 登录后点屏幕右上角的系统状态区（时钟/音量/网络那一块），打开“快捷设置”面板；**Xiaomi Buds 5** 磁贴位于快捷设置图标网格中。点磁贴右侧箭头可选择：
   - 音量滑块（直接控制 Xiaomi Buds 5 的 A2DP sink，显示百分比）
   - 关闭 / 降噪 / 通透
   - 降噪强度（舒适 / 均衡 / 深度）
   - 刷新电量 / 重新连接

服务每 20 秒刷新一次电量，连接断开后会自动重试。耳机控制通道同一时间只允许一个主机占用，因此服务会保持一个长连接；不要同时运行其它会打开 RFCOMM 29 的 Xiaomi 耳机工具。

命令行诊断（可选）：`xiaomi-buds5 status` 查看电量/降噪/编码，`xiaomi-buds5 noise anc|transparency|off` 切换降噪，`xiaomi-buds5 selftest` 自检。它们都通过 D-Bus 调用后端，不直接占用 RFCOMM。

## 高质量 aptX Adaptive 模式

面板里的“音频后端”可以在两种模式之间切换：

- **兼容模式**：原来的 PulseAudio + aptX Classic，最稳妥，可随时回退。
- **高质量模式**：隔离启动 PipeWire 1.7 + WirePlumber + aptX Adaptive。耳机的 `sep9` 会协商为 **aptX Adaptive HQ，48 kHz / 24-bit，12 ms，427 kbps**；协议列表同时提供 **AAC、SBC-XQ、SBC、aptX Classic、aptX Adaptive、Adaptive HQ、Adaptive LL**，均已接入该后端。

切换由 `xiaomi-buds5-adaptive.service` 管理，成功切换后系统会使用私有 Pulse socket；启动失败时服务会直接退出，并只恢复启动前实际运行的那一套声音后端。

> 并发设备：高质量模式仍会枚举板载声卡和 USB 有线耳机；`ACP_PATHS_DIR` / `ACP_PROFILES_DIR` 指向 Ubuntu/Zorin 的 `/usr/share/alsa-card-profile`，否则 PipeWire 1.7 会因默认查找 `/usr/local/share` 而只暴露 `pro-audio` profile。切换后端时只恢复启动前实际运行的那一套声音服务器，禁止 PulseAudio 与 PipeWire 同时抢占 ALSA 设备。

AAC 在高质量模式中由 FDK-AAC 插件提供（系统依赖 `libfdk-aac2` / `libfdk-aac-dev`）；兼容模式的原生 PulseAudio 没有 AAC 编码器，因此协议列表只显示它实际支持的 aptX / SBC / SBC-XQ。aptX Adaptive HQ 是高通 Adaptive 系列的高质量档，**不是严格意义上的 aptX Lossless**；本机当前已验证的是 Adaptive HQ 的实际音频写入链路，不宣称 Lossless。

## 双设备 / 多点连接

Xiaomi Buds 5 的多点连接同一时间只会播放一个音频源，不会让电脑和手机同时出声。为避免两边互相抢占，本工具默认采用共存优先：

- 后台不会因为 RFCOMM 短暂断开而自动 `bluetoothctl connect`；
- 高质量音频栈不再自动连接 A2DP；
- 手机接管音频时，电脑保持 BlueZ 连接，面板显示“蓝牙已连接 · 手机可能在播放”；
- 只有点击面板里的“连接电脑”才会显式把音频切回电脑。

## macOS 风格控制面板

点击顶栏右侧的耳机托盘图标即可打开控制面板（托盘服务随登录自动启动）。面板固定打开在右上角标题栏下方，带有：

- 独立关闭按钮
- 可拖动音量滑杆
- 关闭 / 降噪 / 通透
- 降噪强度
- 蓝牙协议选择

面板是独立窗口，因此拖动音量滑杆不会导致菜单自动关闭。

## 当前已启动的托盘入口

如果刚安装后没看到快捷设置磁贴，可以先看顶栏右侧的**耳机图标**（Zorin AppIndicator 托盘）。它已经作为用户服务启动：

```sh
systemctl --user status xiaomi-buds5-tray.service
```

点耳机图标可以查看电量、切换降噪/通透、调整音量、静音和重新连接。这个托盘会随登录自动启动。

## 没看到快捷设置控件时

先确认扩展状态：

```sh
gnome-extensions info xiaomi-buds5-panel@kang
```

正常应显示 `状态: ENABLED`。如果显示 `ERROR`，注销并重新登录，然后再次检查。GNOME 43 的 Wayland 会话不能像 X11 那样用 `Alt+F2` 输入 `r` 重启 Shell。

## 技术说明

- 传输：Bluetooth Classic RFCOMM，SDP UUID `0000fd2d-0000-1000-8000-00805f9b34fb`。本机 Xiaomi Buds 5 的控制通道是 **29**；代码也会探测其它常见通道。
- 音频编码：兼容模式由 PulseAudio 协商为 **aptX Classic**；高质量模式由项目内隔离的 PipeWire 1.7 + WirePlumber 协商为 **aptX Adaptive HQ / Adaptive / LL**。耳机的 `sep7` 是 aptX Classic；`sep9` 是高通 Adaptive 系列端点。
- aptX Adaptive 来源：基于 `AKS-Logic/aptx-adaptive-linux` 的研究实现，并为 Xiaomi `D740/AD00` 端点补上兼容；Rust 测试 126 项通过。该实现不是 aptX Lossless，面板标注为“接近无损”的高质量档是 Adaptive HQ。
- 运行文件：`audio-adaptive/` 是启停脚本，`runtime/pipewire-build/` 是项目内自包含的 PipeWire/WirePlumber 运行文件，不依赖 `/tmp` 或系统 PipeWire 版本。
- 重新安装：`./install.sh` 会保留自适应运行文件，并把 `xiaomi-buds5-adaptive.service` 注册为按需启动，不会默认替换系统音频后端。
- 帧：`FE DC BA | flags | opcode | len(u16 BE) | payload | EF`
- 认证：Xiaomi/JieLi command auth，`AUTH_CHECK` (0x50) + `AUTH_SEND_CALC_RESULT` (0x51)。认证变换来自 `redmi-buds-linux` 的 MIT 实现，见 `LICENSE.redmi-buds-linux`。
- 电量：`GET_TARGET_INFO` (0x02) 的 multi-battery 记录；同时兼容设备主动上报的 `0x0e/type 4` 电量帧。
- 降噪：`SET_TARGET_INFO` (0x08) 的 `02 04 <mode>`，mode `0=off, 1=ANC, 2=transparency`。固件随后把状态镜像到 config 11。
- 电量未知：充电盒不在附近或耳机取出时，第三项常返回 `0xFF`，这是设备正常行为，不是读取失败。
- 音量：面板跟随当前后端控制默认耳机 sink（兼容模式为 `bluez_sink.*.a2dp_sink`，高质量模式为 `bluez_output.*`）。音量是主机侧 A2DP sink 的音量，不需要额外 RFCOMM 命令，也不会与降噪控制通道争用。
- 高质量模式切换会短暂重连 A2DP；这是 BlueZ/耳机重新协商 SEP 所必需的，不会改动配对信息。

## 卸载

```sh
./uninstall.sh
```

卸载后同样需要注销/重新登录，GNOME 才会移除扩展。
