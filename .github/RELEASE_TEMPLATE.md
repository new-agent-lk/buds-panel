<!--
GitHub Release 正文模板。复制到 Release 编辑框，替换占位内容。
发布流程见 docs/releasing.md。
-->

## Highlights

<!-- 一句话概述本次发布最重要的变化。 -->

- 

## Install

Build from source:

```sh
git clone https://github.com/new-agent-lk/buds-panel.git
cd buds-panel
./install.sh
```

See the [README](README.md) for prerequisites (BlueZ, GTK3, `librsvg2-common`,
PipeWire/PulseAudio). Optional aptX Adaptive stack: [docs/aptx-adaptive.md](docs/aptx-adaptive.md).

## Changes

<!-- 从 CHANGELOG.md 的本版本段落摘录。 -->

See [CHANGELOG.md](CHANGELOG.md).

## Known limitations

- Tested on Xiaomi Buds 5 (R1); other JieLi RCSP earbuds not individually verified.
- aptX Adaptive is out of tree (Qualcomm patent); not bundled in this release.
- The GNOME extension is community-maintained and unverified on GNOME versions > 43.
