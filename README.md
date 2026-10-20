# sushi

一个合成器项目。

## 功能

**声音**

- 双振荡器：Sine / Triangle / Saw / Square / Pulse，支持八度、半音、微调、脉宽、电平
- 锯齿 / 方波 / 脉冲 / 三角均为 PolyBLEP 限带波形，不产生混叠
- 每个振荡器 1–4 重 Unison，带失谐与立体声展宽；每个声部随机起始相位，避免和弦相位抵消
- 噪声、TPT 状态变量滤波器（LP/HP/BP/Notch）带 Drive，共振越高输入越柔和
- 每声部带直流阻断（DC Blocker），波形不会累积直流偏置
- 独立的 Amp / Filter ADSR 包络
- 双 LFO（Sine / Triangle / Saw / Square / S&H），可路由到音高、滤波、音量、脉宽
- 每声部模拟漂移（Drift），Poly / Mono / Legato、Glide
- **Character**：Silk / Grit 两档性格，统一改变饱和、驱动和漂移
- **宏控制**：Brilliance / Motion / Space，每个同时带动一组参数
- 力度同时影响音量与滤波器

**可玩性**

- **琶音器**：Up / Down / UpDown / Random / As Played，1/4–1/32，1–3 八度，跟随宿主速度
- 琶音器还带 **Gate**、**Latch**、**Swing**（0–75%，长短交替且平均不变）、**Ratchet**（每步 1–4 次重触发）与 **Length**（1–16 步）
- 琶音器面板右侧有 16 颗 **POS** 指示灯，亮起的一颗就是当前所在的步（不随 Ratchet 的重触发跳动）
- **效果链**：Chorus（湿声低通）、Delay（反馈低通 + 软削波）、Reverb（15 ms 预延迟 + 输入低通）；混音为 0 时完全旁路
- **14 个工厂预设** + **MUTATE** 随机演化
- 小示波器、ACT / CLIP / 声部 LED、琶音 LED

**界面**

- 平面（SH-101 蓝图风格）单页面板：彩色分区标题、1 px 细线、竖向推子（黑色滑块完全嵌入凹槽）、所有参数可精确输入数值、波形/模式点击弹出
- 双击推子、开关、性格按钮或弹窗，即可将参数恢复到出厂默认值
- 4 行模块化分区：OSC / FILTER / MIX · ENV / LFO · ARP / VOICE · FX / MACROS / OUTPUT
- 标题栏的 **PATCHES** 按钮打开补丁浏览器：选择目录、列出补丁、读取与保存（同名文件需连按两次 SAVE 才会覆盖）
- LCD 显示当前声音的名字：用户补丁显示补丁名，工厂预设显示编号与预设名，被改动过时名字后带 `*`

## 补丁（Patch）

补丁是纯 JSON 文本，任何语言都能读写。保存时写出全部 65 个参数，读取时先回到出厂默认值，
再写入文件中出现的参数——缺失的参数保持默认，未知参数只警告不报错，因此新旧版本互相兼容。

```json
{
  "format": "sushi-patch",
  "version": 1,
  "name": "JUNO PAD",
  "params": {
    "osc1_wave": "Saw",
    "osc1_octave": 0,
    "osc1_fine": 0.0,
    "cutoff": 2400.0,
    "filter_mode": "Lowpass",
    "arp_pattern": "Up",
    "arp_rate": "1/16",
    "poly_mode": "Poly",
    "arp_on": false,
    "volume": 0.5
  }
}
```

- `format` / `version` / `name` 可省略；没有 `params` 时，整个根对象就是参数表，方便手工编辑
- 参数 id 为稳定名称，值都是**普通值**：浮点为 JSON 数字、整数为 JSON 整数、布尔为 `true`/`false`（也接受 `0`/`1`）
- 枚举可写显示名（大小写不敏感，如 `Saw`、`Lowpass`、`1/16`、`As Played`、`S&H`）或整数序号
- 浮点超出范围会被夹到范围内；非有限数值会报错
- `volume` 是线性增益（`1.0` = 0 dB，`0.5` ≈ −6 dB），其余单位与面板一致（Hz、ms、%、音分）

默认补丁目录：**`~/Documents/Sushi Patches`**（找不到 HOME 时用 `sushi-patches`）。在标题栏点
**PATCHES** 打开补丁浏览器，**FOLDER** 里可以填别的目录（`~/` 会展开成 HOME），按 **SET** 生效并
写入配置文件；列表里点一个名字即可读取，**NAME** + **SAVE** 则把当前声音写成补丁。用户选择的目录保存在
配置文件 `$XDG_CONFIG_HOME/sushi/config.json`（未设置时为 `~/.config/sushi/config.json`）：

```json
{
  "patch_dir": "/home/user/Documents/Sushi Patches"
}
```

## 工厂预设

共 14 个：INIT PAD · GLASS KEYS · JUNO PAD · PROPHET BRASS · JUPITER STRINGS · VANGELIS PAD ·
SILK STRINGS · WIDE BRASS · MOOG BASS · ACID LINE · GRIT BASS · PLUCK · MOTION PAD · ARP TOY

工厂预设就是内嵌的补丁 JSON，与用户保存的补丁用同一套读取代码。标题栏的 `<` / `>` 切换预设，
**MUTATE** 在当前声音上做随机变化。

## 构建

```sh
cargo build --release                        # 独立程序：target/release/sushi
cargo nice-plug bundle sushi --release       # CLAP + VST3 + 独立程序
```

产物位于 `target/bundled/`，把 `sushi.clap` 放到 `~/.clap`、`sushi.vst3` 放到
`~/.vst3`，即可在 Reaper 等 DAW 中加载。

离线试听：

```sh
cargo run --release --example render_demo
pw-play target/demo.wav
```
