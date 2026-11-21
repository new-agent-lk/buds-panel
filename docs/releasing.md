# 发布流程

本项目用 [Semantic Versioning](https://semver.org/)，版本号在 `Cargo.toml` 的
`[workspace.package] version` 单一处维护（三个 crate 都 `version.workspace = true` 继承它）。
发版记录写在 `CHANGELOG.md`（[Keep a Changelog](https://keepachangelog.com/) 格式）。

## 一次发布的 9 个步骤

在干净、已同步的 `main` 上，依次执行：

1. **格式检查**
   ```sh
   cargo fmt --all --check
   ```

2. **Lint（零警告）**
   ```sh
   cargo clippy --all-targets -- -D warnings
   ```

3. **构建**
   ```sh
   cargo build --release
   ```

4. **测试**
   ```sh
   cargo test
   ```

5. **脚本语法**
   ```sh
   bash -n install.sh uninstall.sh audio-adaptive/*.sh
   ```

6. **干净安装冒烟测试**（临时 `HOME` / 隔离 D-Bus，避免碰真实会话）
   ```sh
   TMP=$(mktemp -d)
   HOME="$TMP" ./install.sh
   # 期望：服务起、bus name 被占用、tray 注册 SNI、
   #       busctl get-property io.github.kang.XiaomiBuds5 ... 各属性正常
   HOME="$TMP" ./uninstall.sh
   rm -rf "$TMP"
   ```

7. **更新 `CHANGELOG.md`**：把 `## [Unreleased]` 收进新的 `## [x.y.z] - YYYY-MM-DD`
   段，分组 Added / Changed / Fixed / Removed；补好底部的 compare 链接。

8. **改版本并打 annotated tag**（版本只改一处）
   ```sh
   # 编辑 Cargo.toml 的 [workspace.package] version = "x.y.z"
   cargo build --release            # 刷新 Cargo.lock
   git add -A && git commit -m "chore(release): vX.Y.Z"
   git tag -a vX.Y.Z -m "vX.Y.Z"
   ```
   预发版用 `vX.Y.Z-rc.N`。

9. **建 GitHub Release**：以 tag `vX.Y.Z` 为基准，Release 正文用
   `.github/RELEASE_TEMPLATE.md` 的结构（Highlights / Install / Changes /
   Known limitations）。v0.1 只发源码（GitHub 自动 source tarball + CHANGELOG 摘要），
   **不挂预编译 aptX 资产**（见 `docs/aptx-adaptive.md`）。

## 备注

- 提交信息用 [Conventional Commits](https://www.conventionalcommits.org/)。
- 发布前一并确认：README 徽章/screenshots 未过期、`NOTICE` 与第三方版本一致、
  `LICENSE` 年份正确。
