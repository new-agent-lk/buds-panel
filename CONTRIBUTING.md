# 贡献指南

感谢你对 Buds Panel 的兴趣！这是一个小型的单维护者开源项目，欢迎 Issue 与 Pull Request。

## 治理

本项目采用 **BDFL**（仁慈独裁者）模式，由单一维护者决策。所有 Issue 与 PR 公开讨论；
不设 CLA，也不强制 DCO 签署。提交 PR 即表示你同意以本项目的 Apache-2.0 许可授权你的贡献。

## 报告问题

- 提交前先搜索现有 Issue，避免重复。
- 用 bug report 模板，附上：发行版与版本、GNOME/桌面环境、PipeWire 或 PulseAudio 版本、
  耳机型号、`xiaomi-buds5 status` 输出，以及相关 `journalctl --user -u xiaomi-buds5-panel` 日志。
- **请勿**粘贴个人隐私信息（真实 MAC 可打码、去掉主机名/用户名路径）。

## 提交 Pull Request

1. Fork 并从 `main` 拉分支。
2. 改动前先读相关代码；保持改动聚焦、最小。
3. 提交信息遵循 [Conventional Commits](https://www.conventionalcommits.org/)
   （`feat:` / `fix:` / `docs:` / `refactor:` / `chore:` …）。
4. 确保以下检查全部通过（PR 的 CI 也会跑同样的检查）：

   ```sh
   cargo fmt --all --check
   cargo clippy --all-targets -- -D warnings
   cargo build --release
   cargo test
   bash -n install.sh uninstall.sh audio-adaptive/*.sh
   ```

5. 在 PR 描述里说明：改了什么、为什么、如何验证、影响的组件（service / tray / cli / 扩展 / audio-adaptive）。
6. 涉及硬件行为（电量、降噪、编码、音频路由）的改动，请注明你实测的耳机型号与结果。

## 代码风格

- 遵循项目现有结构（分层、命名、错误处理），不要为“以后可能用到”加抽象或配置。
- 只在代码无法自解释时写一行注释说明 *为什么*。
- 不带入新的第三方依赖，除非确有必要并在 PR 中说明理由。

## 许可

贡献的代码以 Apache-2.0 授权。若你的改动包含来自其他项目的代码，请在 PR 中注明来源与许可，
并同步更新 NOTICE。
