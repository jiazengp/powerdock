# PowerDock

![开发预览](https://img.shields.io/badge/status-0.3.0--preview.2-orange)
![Windows 11 x64](https://img.shields.io/badge/Windows-11%20x64-0078D4)
![License: MIT](https://img.shields.io/badge/license-MIT-green)

Windows 原生的外设电量托盘工具。Rust 在后台监测鼠标、键盘、耳机和手柄的电量，C++ / WinUI 3 按需显示原生面板；不使用 WebView，也不依赖 .NET 应用运行时。

项目处于开发预览阶段，测试版可在 [Releases](https://github.com/jiazengp/powerdock/releases) 下载。

> **非官方项目 / Unofficial project**
>
> PowerDock 是独立开源项目，与 MCHOSE（迈从）、Sennheiser（森海塞尔）、Logitech（罗技）、SteelSeries（赛睿）及其他外设厂商没有隶属、授权、赞助或背书关系。所有产品名称和商标归各自所有者。
>
> PowerDock is an independent open-source project. It is not affiliated with, endorsed by, or sponsored by any device manufacturer. All product names and trademarks belong to their respective owners.

## 功能

- **托盘优先**：显示当前在线设备中电量最低的一台；悬停或点击即可查看完整列表。
- **多设备**：按设备分别查询、排序、缓存和提醒，不把接收器状态误当成设备在线状态。
- **多来源**：支持 Windows 电量属性、标准 BLE Battery Service、只读 HID / HID++、RFCOMM 等路径。
- **低打扰提醒**：支持系统通知和游戏内悬浮提醒，可为每台设备单独设置阈值或关闭提醒。
- **原生界面**：WinUI 3 面板、系统主题、减少动画和中文 / English 切换。
- **只读访问**：只发送身份和电量查询，不修改灯效、DPI、按键或其他设备配置。
- **本地优先**：没有遥测、账号或上传功能；诊断 ZIP 由用户自行决定是否分享。

## 系统要求

- Windows 11 x64 build 26100（24H2）或更高版本；不支持 Windows 10、ARM64、macOS 或 Linux。
- 构建需要 Rust 1.99.0、Visual Studio 2026 C++ 桌面工具、Windows SDK 26100 和 PowerShell 7。
- 运行时使用项目自带的 WinUI 运行时文件，不需要安装 .NET 桌面运行时。

## 下载

首次试用建议选择 [Releases](https://github.com/jiazengp/powerdock/releases) 中的 `PowerDock-win-x64-preview-Portable.zip`。解压到固定目录，运行顶层 `PowerDock.exe`，保留同级的 `current\` 文件夹和 `Update.exe`，以便后续更新。

`PowerDock-win-x64-preview-Setup.exe` 是安装器测试包，供参与安装流程测试的用户使用。下载文件的 SHA-256 可用发布页附带的 `SHA256SUMS` 或同名 `.sha256` 文件核对。

## 构建

```powershell
git clone https://github.com/jiazengp/powerdock.git
cd powerdock
pwsh -NoProfile -File scripts/build.ps1
```

`scripts/build.ps1` 会先执行检查，再构建 Rust 后台和 WinUI 前端。构建结果位于 `target\release\`；运行 `target\release\powerdock.exe` 时保留同级的 `ui\` 文件夹。

需要便携包时：

```powershell
pwsh -NoProfile -File scripts/package.ps1 -MetadataPath target/package.json
pwsh -NoProfile -File scripts/test-package.ps1 -MetadataPath target/package.json
```

便携包输出到 `dist\`，包含完整运行时、应用资源和许可，不包含内部文档、PDB 或中间构建文件。

## 使用

启动后程序常驻通知区域。单击托盘图标打开设备页，右击菜单进入设备、设置或退出；关闭面板不会停止后台。默认每 60 秒刷新，可选 30、60 或 120 秒。登录启动由用户主动开启。

首次启动时，只有在当前设置不存在的情况下，才会尝试迁移旧版 MchoseBattery 配置；旧配置不会被覆盖。设置和错误日志位置见下文“设备访问与隐私”。

常用验证参数：

```text
powerdock.exe --version
powerdock.exe --show
powerdock.exe --preview
powerdock.exe agent capabilities
powerdock.exe agent devices list
```

Release 是 Windows GUI 子系统，PowerShell 直接调用时可能不等待或收不到标准输出；需要 JSON 时可使用 `Start-Process -Wait` 并重定向输出。

## 设备访问与隐私

- 只读查询身份与电量，不向设备写入配置。
- 诊断输出不包含设备路径或序列号；导出 ZIP 在本地生成，是否分享由用户决定。
- `%LOCALAPPDATA%\PowerDock\settings.json` 保存设置与提醒去重记录。
- `%LOCALAPPDATA%\PowerDock\errors.log` 只记录错误，约 64 KiB 后重置，不记录设备路径或序列号。
- 开始菜单的 `PowerDock.lnk` 用于 Windows 通知身份；登录启动写入当前用户 Run 项 `PowerDock`。

移动未签名便携程序可能生成新的托盘身份。建议从固定路径运行，不要只复制单个 EXE。

## 已知限制

- 当前构建未签名；Windows SmartScreen 可能提示未知发布者。
- 安装器的干净安装、覆盖升级和卸载仍待实机验收。
- 能否读取电量取决于设备、连接方式和驱动；实验性协议默认关闭，需要在设置中开启。
- 某些厂商软件、独占游戏或驱动更新可能占用设备接口，导致暂时读不到电量。
- 新增型号必须用真实硬件验证；模拟数据或云端 CI 不能证明硬件兼容。

## Agent CLI

使用 `powerdock.exe agent capabilities` 查看命令和字段说明。`agent status`、`agent devices list` 和 `agent devices catalog` 读取后台现有快照，支持刷新、设置读写和窗口控制；没有 MCP、HTTP 服务或额外常驻线程。

## 项目结构

```text
src/              Rust 后台、设备发现、协议适配、CLI 和更新逻辑
ui/               C++ / WinUI 3 原生界面
assets/           应用图标和图标许可
locales/          中文和英文字符串
scripts/          构建、检查、打包和测试脚本
build_support/    构建期资源生成
vendor/           上游协议代码和第三方许可
```

## 参与贡献

- 开发与发布流程见 [CONTRIBUTING.md](CONTRIBUTING.md)。
- 新增设备或提交实机结果见 [CONTRIBUTING-DEVICES.md](CONTRIBUTING-DEVICES.md)。
- 第三方代码来源和许可见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。

内部验收资料暂不进入公开仓库；提交 PR 时请把可公开复现的验证结论写进 PR 描述，并隐去设备昵称、MAC、序列号和原始设备路径。

## 许可证

PowerDock 使用 MIT License，详见 [LICENSE](LICENSE)。产品名称与商标归各自所有者；本项目与设备厂商无隶属或背书关系。
