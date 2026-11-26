# 贡献设备兼容性

可以提交实机结果，也可以新增设备适配。先检查有无可用的读取方式。

## 1. 确认设备和现有路径

记录品牌、完整型号、固件、Windows 内部版本号、PowerDock 版本和连接方式。USB、接收器和蓝牙分开测，无法确认的型号或固件写「未知」。

在 `powerdock.exe` 所在目录打开 PowerShell。自更新便携版用 `current` 里的程序。前 3 条可以直接运行，后 2 条需要 PowerDock 正在运行。

```powershell
.\powerdock.exe agent capabilities
.\powerdock.exe agent compatibility
.\powerdock.exe agent diagnose --bluetooth
.\powerdock.exe agent devices list --all
.\powerdock.exe agent providers list
```

`compatibility` 显示当前 HID 接口的 VID/PID、Usage、报告长度、描述符、实验开关和不匹配原因。分享时保留排查所需的 VID/PID、Usage 和报告长度，删去设备昵称、MAC、序列号、设备路径和可关联报告的设备 ID。

## 2. 选择最小接入方案

| 已有证据 | 优先做法 |
| --- | --- |
| Windows、标准 BLE 或标准 HID 已提供电量 | 先试现有读取方式，再按实际问题修发现、匹配或解析。 |
| 同系列设备共用协议 | 用公开规范或实机报文核对请求、回复和适用范围，再复用读取代码。 |
| 新品牌或不同协议 | 记录规范、上游版本、许可、Windows 读取方式和已知副作用。证据不足时先提交调查结果。 |
| 新设备类型或电池组件 | 沿用数据模型，说明设备类别、连接方式、电量精度和电池用途。 |

按型号、连接方式、接口能力及 VID/PID 或已验证的协议身份，限定适配范围。只发送经过验证的只读查询。复用代码或资料时核对许可，并更新 [第三方许可](THIRD_PARTY_NOTICES.md)。

## 3. 接入现有代码

- 在 [`src/adapters/`](src/adapters/) 维护协议和设备匹配。设备发现和 `agent compatibility` 共用这些规则。
- Windows HID 状态查询复用 [`device.rs`](src/adapters/device.rs) 的句柄和 [`Reply`](src/adapters/device/overlapped.rs)。先开始读取，再发送查询。提交、等待和完成都用同一截止时间并检查会话。
- 复用 [`discovery.rs`](src/monitor/discovery.rs) 和 [`reader.rs`](src/monitor/reader.rs) 的发现、去重、会话和调度。保留超时、取消、同接收器串行查询和标准来源回退。
- 按协议校验回复关联、长度、报告 ID、校验和、数值范围。用 [`battery.rs`](src/battery.rs) 表达实报百分比、粗档、估算、缓存和未知值。
- 实验开关沿用 [`providers.rs`](src/settings/providers.rs)。适用范围要精确，尚待实测的专用读取方式默认关闭。

## 4. 验证并提交结果

1. 测试匹配与解析，覆盖相近但不匹配的接口、截断或错误回复、掉线和取消后的迟到回复。注明数据来自实机还是模拟。
2. 运行 `pwsh -NoProfile -File scripts/check.ps1`。按改动范围补跑 Release 构建、界面或 Agent CLI 检查。
3. 在实机上和厂商软件等独立来源对照电量，记录来源、精度和条件。检查充电、重连、唤醒、切换连接方式及厂商软件同时运行。未覆盖的情况标为「待测」。
4. 在 PR 写明已测和待测项，附上命令、版本、退出码和去掉设备标识的结果。实机读到有效电量后，再标为已支持。

只提交反馈时，提供设备信息、操作步骤、预期和实际结果，以及去掉设备标识的输出。提交代码时再补协议来源和受影响的型号。
