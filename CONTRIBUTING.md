# 开发与贡献

提交设备实测结果或新增适配，请先看 [CONTRIBUTING-DEVICES.md](CONTRIBUTING-DEVICES.md)。

## 环境

- Windows 11 x64 build 26100 或更高版本。
- Rust 1.99.0，包含 rustfmt 和 Clippy。
- Visual Studio 2026 C++ 桌面工具、v145 工具集和 Windows SDK 26100。
- PowerShell 7；构建时需要访问 crates.io 和 NuGet。

```powershell
git clone git@github.com:jiazengp/powerdock.git
cd powerdock
pwsh -NoProfile -File scripts/build.ps1
```

`scripts/check.ps1` 执行翻译资源生成回归、Rust 格式、Clippy 和单元测试。`scripts/build.ps1` 先运行同一套检查，再构建 Rust Release 和 WinUI；所有原生命令失败都会终止构建。

## 代码约定

- 优先复用现有发现、去重、会话和 I/O 调度。新增设备协议必须保留接口白名单、正常应答校验和取消安全。
- 只实现基于真实证据的只读电量 / 身份查询；不要向未知接口发送猜测命令，也不要增加未经实测的灯效、DPI、按键或固件写命令。
- 界面只通过现有 IPC 与后台交互。不要为单个型号预建多品牌框架，也不要把标准鼠标 / 键盘集合当成配置接口。
- 翻译统一维护在 `locales/en-US.json` 和 `locales/zh-CN.json`。两种语言必须保持相同的键和命名参数；WinUI 资源由 `scripts/generate-resources.ps1` 生成，不直接编辑 `.resw`。
- 移动未签名便携程序会改变托盘身份；测试时使用固定目录，避免把环境问题误判为代码回归。

## 目录

| 路径 | 内容 |
| --- | --- |
| `src/`、`ui/` | Rust 后台和 C++ / WinUI 3 界面。品牌协议和传输集中在 `src/adapters/`。 |
| `assets/`、`locales/` | 应用图标、语言资源和相关许可。 |
| `scripts/`、`build_support/` | 构建、检查、打包、测试和构建期资源生成。 |
| `vendor/` | 复用的上游代码、固定来源和分发所需许可。 |
| `target/`、`dist/`、`app/` | 本地构建、分发和试用目录，不提交。 |

内部调研、验收证据、实施计划和临时实验不进入公开仓库。需要公开的兼容性结论请写入 PR 描述。

## 检查与 CI

CI 在 `main` 推送、PR 和手动触发时执行：Rust 格式、Clippy、单元测试、Rust / WinUI Release 构建、更新包验证、便携包校验，以及打包后的 `--version`、`--diagnose`、图标导出和 Agent CLI 检查。托管 runner 使用 `windows-2025-vs2026`。

CI 没有真实鼠标、耳机、手柄或桌面主题，因此无法证明硬件兼容。设备协议改动必须在 PR 中说明实机型号、连接方式、固件、已测能力、未测项和脱敏后的结果。无法复现的状态标记为待测，不要根据模拟测试宣称支持。

## 发布

准备发布时：

1. 更新 `Cargo.toml` 版本和 `CHANGELOG.md`，核对本次涉及的设备验证结果。
2. 运行 `scripts/check.ps1`、`scripts/build.ps1`、`scripts/package.ps1` 和 `scripts/test-package.ps1`。
3. 完成涉及本次改动的实机验收，再提交并等待 `main` 的 CI 通过。
4. 创建与 Cargo 版本完全一致且带 `v` 前缀的标签，例如 `v0.3.0-preview.2`；不要移动已发布的标签。
5. `release.yml` 会复用 CI，校验版本、SHA-256、便携包、Setup 和更新 feed，然后创建草稿 Release。人工核对后再发布。

Setup 沿用锁定的 Velopack 工具，不引入单独的安装器框架：

```powershell
pwsh -NoProfile -File scripts/package.ps1 -MetadataPath target/package.json
pwsh -NoProfile -File scripts/package-updates.ps1 -MetadataPath target/package.json
pwsh -NoProfile -File scripts/test-managed-package.ps1
pwsh -NoProfile -File scripts/test-setup-package.ps1
```

当前程序和安装包未签名。干净系统安装、覆盖升级和卸载仍需单独实机验证，不能只看打包脚本通过。

## 来源与许可

新增或更新依赖、协议实现或图标时，同步核对具体文件许可，并更新 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) 和 `vendor/` 中的原始声明。Dependabot 每月分组提出依赖更新，不自动合并。
