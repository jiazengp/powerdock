# 开发与贡献

提交设备实测结果或新增适配，见 [设备贡献指南](CONTRIBUTING-DEVICES.md)。

## 环境

- Windows 11 24H2（x64）或更新版本。
- Rust 1.99.0，包含 rustfmt 和 Clippy。版本由 `rust-toolchain.toml` 固定。
- Visual Studio 2026 的 C++ 桌面开发工具（v145）和 Windows SDK 26100。
- PowerShell 7。构建需要访问 crates.io 和 NuGet。

## 构建

```powershell
git clone https://github.com/jiazengp/powerdock.git
cd powerdock
pwsh -NoProfile -File scripts/build.ps1
```

`scripts/build.ps1` 先检查翻译、Rust 格式、Clippy 和单元测试，再构建 Rust 后台和 WinUI 界面。只跑检查时用 `scripts/check.ps1`。

构建完成后可运行 `target\release\powerdock.exe`，保留同级的 `ui` 文件夹。

## 代码约定

- 复用现有的设备发现、去重、会话和 I/O 调度。新增协议要校验接口和回复，并处理取消。
- 电量适配只发送经过验证的身份和电量查询。
- 界面沿用现有 IPC。单型号适配优先复用现有模块。
- 翻译维护在 `locales/en-US.json` 和 `locales/zh-CN.json`。两份文件的键和参数名要一致，`.resw` 由 `scripts/generate-resources.ps1` 生成。
- 测试便携版时，从固定目录启动。换目录可能改变 Windows 识别到的托盘身份。

## 目录

| 路径 | 内容 |
| --- | --- |
| `src/` | Rust 后台、命令行和更新。设备适配在 `src/adapters/`。 |
| `ui/` | C++ / WinUI 3 界面。 |
| `assets/`、`locales/` | 图标、许可和中英文文案。 |
| `scripts/`、`build_support/` | 构建、检查、打包和资源生成。 |
| `vendor/` | 第三方代码、来源和许可。 |
| `target/`、`dist/`、`app/` | 本地构建、打包和试用目录。 |

调研、验收和实验资料留在本地，可公开的验证结果写进 PR。

## 命令行

在 `powerdock.exe` 所在的目录打开 PowerShell。自更新便携版用 `current` 里的程序。

```powershell
.\powerdock.exe --version
.\powerdock.exe --show
.\powerdock.exe --preview
.\powerdock.exe agent capabilities
.\powerdock.exe agent devices list
```

`--preview` 用模拟数据打开界面。`agent status`、`agent devices list` 和 `agent devices catalog` 读取正在运行的 PowerDock 的数据。Agent CLI 也可刷新电量、改设置或控制窗口。实验性读取默认关闭，相关开关见 `agent capabilities`。

要把 JSON 存到文件，可用 `Start-Process .\powerdock.exe -ArgumentList 'agent', 'capabilities' -Wait -RedirectStandardOutput capabilities.json`。

## 本机配置

- 设置默认保存在 `%LOCALAPPDATA%\PowerDock\settings.json`，日志在同目录的 `logs` 文件夹。
- 登录时启动使用当前用户的 `PowerDock` 启动项。Windows 通知使用开始菜单里的 `PowerDock.lnk`。

## 检查与 CI

推送 `main`、提交 PR 或手动运行都会触发 CI。CI 会检查代码、构建程序并验证更新包和便携包，完整步骤见 `.github/workflows/ci.yml`。

设备协议改动要在真实设备上测试，按 [设备贡献指南](CONTRIBUTING-DEVICES.md) 把已测结果和待测项写进 PR。

## 发布

1. 更新 `Cargo.toml` 版本和 `CHANGELOG.md`。
2. 构建程序，再打包并运行下列检查。
3. 完成本次改动涉及的实机验证，等待 `main` 的 CI 通过。
4. 推送与 `Cargo.toml` 版本一致、带 `v` 前缀的新标签。
5. `release.yml` 会校验版本、校验值、便携包、安装器和更新清单，再创建草稿。核对草稿后发布。

安装器和更新包使用仓库锁定的 Velopack，通过 .NET SDK 还原工具。

```powershell
pwsh -NoProfile -File scripts/package.ps1 -MetadataPath target/package.json
pwsh -NoProfile -File scripts/test-package.ps1 -MetadataPath target/package.json
dotnet tool restore --configfile NuGet.Config
pwsh -NoProfile -File scripts/package-updates.ps1 -MetadataPath target/package.json
pwsh -NoProfile -File scripts/test-managed-package.ps1
pwsh -NoProfile -File scripts/test-setup-package.ps1
```

打包文件默认在 `dist/`。安装器需在实机上验证首次安装、覆盖升级和卸载。

## 来源与许可

更新依赖、协议实现或图标时，核对具体文件的许可，在 `vendor/` 保留原始声明，并更新 [第三方许可](THIRD_PARTY_NOTICES.md)。Dependabot 每月分组提出依赖更新，由维护者审核合并。
