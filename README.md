# SUSHI

复音软件合成器。**CLAP / Linux**。

## 装

```bash
cargo build --release
install -m 755 target/release/libsushi.so ~/.clap/sushi.clap
```

或者 `cargo nice-plug bundle sushi --release`，产物在 `target/bundled/`。
REAPER 里 `Options → Preferences → Plug-ins → CLAP → Re-scan`。

## 界面

黑底、单一橙色强调、绿色读数，全部平角 1 px 描边，120 fps。

- **顶栏**：预设 LCD（点开就是 patch 浏览器）、`<` `>`、`MUTATE`、`MOD` / `FX` 标签、
  声部表、示波器、`ACT` / `CLIP`
- **主页面**四条：`VCO 1 | VCO 2 | PULSE WIDTH | SOURCE MIXER`、
  `VCF | MOD ENV | AMP ENV`、`ARP | VOICE`、`MACROS | VCA`
- **MOD 页**：左边路由表 + Target 检查器（当前值、调制范围、实时贡献），
  右边 LFO 1 / LFO 2 / MOD ENV / AMP ENV / MACROS 一列
- **FX 页**：左边八个可拖动的链块（八级任意排序）+ `GLOBAL WET` + `CHAIN PRESET`，
  右边 2×4 模块网格
- **底部键盘**：点/拖滑音，力度看点击高度；`A S D F G H J K L ; '` 白键、
  `W E T Y U O P` 黑键、`Z` / `X` 换八度

操作：

| 操作 | 效果 |
|---|---|
| 拖控件 | 改值 |
| 滚轮 / Shift+滚轮 | 改值 / 微调 |
| 右键控件 | 输入数值 |
| 双击控件 | 复位 |
| **空白处右键** | **界面缩放 50–300%**（档位是上限，窗口更大也不拉伸） |

## 声音（Legacy 引擎）

- 两个 PolyBLEP 带限振荡器（正弦 / 三角 / 锯齿 / 方波 / 脉冲）、低八度方波 sub、
  hard sync、phase FM、ring mod、最多 4 声 unison
- 16 声部；poly / mono / legato，glide、velocity、drift、每声部元件容差
- 12 / 24 dB 梯形状态变量滤波器（LP / HP / BP / Notch）+ 全局 12 dB HPF，
  cutoff / res / env / keytrack / drive
- 两个 ADSR（带曲线）、两个 LFO（自由或同步）、8 槽调制矩阵（双极性）
- 琶音器：Up / Down / Up-Down / Random / As Played，1/4–1/32、1–3 八度、
  swing、gate、ratchet、length、latch、chord、strum、probability
- 八级立体声效果链，**任意顺序**（Drive、Phaser、Chorus、Delay、Reverb、EQ、
  Compressor、Tremolo），六个链预设 + 全局 wet
- 140 个参数、11 个出厂预设；patch 是 JSON，参数 ID 稳定

## 补丁

用户 patch 不在这个仓库，放在 REAPER 工程仓：

    ~/Music/all_my_reaper/presets/Sushi/patches

或者在 `~/.config/sushi/config.json` 里指定 `patch_dir`。
只有必须随插件发布的内容放 `factory/`。

## 开发

```bash
cargo test          # 327 个测试：DSP、几何、文本不重叠
cargo build --release
```

## License

MIT。内置 Quicksand / JetBrains Mono 字体为 SIL OFL 1.1。
