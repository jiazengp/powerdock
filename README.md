# PowerDock

![开发预览](https://img.shields.io/badge/status-0.3.0--preview.4-orange)
![Windows 11 x64](https://img.shields.io/badge/Windows-11%20x64-0078D4)
![License: MIT](https://img.shields.io/badge/license-MIT-green)

PowerDock 是 Windows 原生的外设电量工具。在任务栏右下角就能看电量，电量低了会提醒你充电。

> **非官方项目 / Unofficial project**
>
> PowerDock 是独立开源项目。产品名称和商标归各自所有者。
>
> PowerDock is an independent open-source project. Product names and trademarks belong to their respective owners.

## 功能

- **托盘电量**。图标显示在线设备里最低的电量，悬停可看摘要。
- **多设备**。同时查看鼠标、键盘和耳机，可按电量、类型或充电状态排序。
- **电量来源**。可读取 Windows 提供的电量，也能直接查询兼容设备。
- **低电量提醒**。在设备卡片的 **⋯ → 电量提醒** 里设提醒电量或关闭提醒。支持 Windows 通知和游戏内弹窗。
- **原生界面**。支持中英文、主题切换和减少动画。

## 系统要求

需要 Windows 11 24H2（x64）或更新版本。程序自带运行所需的文件。

## 下载

首次试用可在 [发布页](https://github.com/jiazengp/powerdock/releases) 的 **Assets** 下载 `PowerDock-win-x64-preview-Portable.zip`。解压到固定目录，双击最外层的 `PowerDock.exe`。保留旁边的 `current` 文件夹和 `Update.exe`，更新时会用到。

`PowerDock-win-x64-preview-Setup.exe` 用于测试安装、升级和卸载。下载后可用发布页的 `SHA256SUMS` 或对应的 `.sha256` 文件核对。

## 使用

点右下角的 PowerDock 图标打开设备页，右键可进设置或退出。关掉窗口后仍会查电量。默认每 60 秒刷新，在 **设置 → 常规** 可改刷新周期或开启 **登录时启动**。

如果电脑上只有旧版 MchoseBattery 的设置，PowerDock 会尝试导入。

检查和下载更新默认使用 Windows 系统代理。可在 **设置 → 关于** 调整 **跟随 Windows 系统代理**。

## 设备访问与隐私

- PowerDock 只读取设备信息和电量。设置和日志都保存在本机。

## 已知限制

- 测试版未签名，Windows SmartScreen 可能提示发布者未知。
- 安装器的首次安装、覆盖升级和卸载仍待实机验证。
- 能否读取电量取决于设备型号、连接方式和驱动。
- 厂商软件占用设备接口时，可能暂时读不到电量。

读不到电量时，请确认设备已开机并连上电脑，再点 **刷新所有设备**。如果仍有问题，请带上型号、连接方式和复现步骤。[反馈问题](https://github.com/jiazengp/powerdock/issues)

## 参与贡献

- [开发指南（构建、项目结构和命令行）](CONTRIBUTING.md)
- [提交实测结果或新增设备适配](CONTRIBUTING-DEVICES.md)
- [第三方代码与许可](THIRD_PARTY_NOTICES.md)

## 许可证

PowerDock 使用 [MIT 许可证](LICENSE)。
