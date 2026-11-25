# 贡献设备兼容性

你可以只提交实机结果，也可以提交代码扩展现有型号、连接方式或设备类型。先用下方命令检查现有适配路径。

## 1. 确认设备和现有路径

记录品牌、完整型号、固件、Windows build、PowerDock 版本和连接方式。分别测试 USB、2.4 GHz 接收器和蓝牙；固件或零售型号无法确认时写“未知”。

用实际 `powerdock.exe` 取证。托管便携包使用 `current/powerdock.exe`。下面前三个命令可以独立运行，最后两个读取当前登录会话中后台实例的快照。

```text
powerdock.exe agent capabilities
powerdock.exe agent compatibility
powerdock.exe agent diagnose --bluetooth
powerdock.exe agent devices list --all
powerdock.exe agent providers list
```

`compatibility` 只枚举当前存在的 HID 接口，显示 VID/PID、Usage、报告长度、描述符、实验开关和拒绝原因。分享前删去设备昵称、MAC、序列号、原始设备路径和可跨报告关联的设备 ID；保留复现所需的 VID/PID、Usage 和报告长度。

## 2. 选择最小接入方案

| 已有证据 | 优先做法 |
| --- | --- |
| Windows 已提供电量、设备有标准 BLE Battery Service 或 HID BatteryStrength Feature | 先验证现有后端和来源选择，只修证据指向的发现、匹配或解析问题。 |
| 同系列设备使用相同协议 | 用公开规范或实机报文确认指令、回复布局和适用范围，复用已有 reader。 |
| 新品牌或不同协议 | 记录规范 / 上游文件、固定版本、许可证、Windows 传输和已知副作用；证据不足时先提交调查记录。 |
| 新设备类型或电池组件 | 分开描述设备类别、连接方式、精度和电池角色，复用现有数据模型。 |

适配范围应落实到型号、VID/PID 或已验证的协议身份、连接方式及所需接口能力。只构造有证据的只读电量和状态查询；配对、固件、灯效和其他设备配置不属于电量适配范围。复用的代码和资料要核对具体文件许可，并同步更新 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。

## 3. 接入现有代码

- 在 [`src/adapters/`](src/adapters/) 的品牌或标准模块维护协议和设备匹配。生产发现和 `agent compatibility` 共用这些判断。
- 对使用 Windows HID 状态接口的设备，复用 [`device.rs`](src/adapters/device.rs) 的句柄和 [`Reply`](src/adapters/device/overlapped.rs)。先启动读取再发送查询，用同一截止时间和会话检查覆盖提交、等待和完成。
- 复用 [`src/monitor/discovery.rs`](src/monitor/discovery.rs) 和 [`src/monitor/reader.rs`](src/monitor/reader.rs) 的发现、去重、会话和调度。保留期限、撤销、同接收器串行和标准来源回退。
- 按协议校验回复关联、长度、报告 ID、校验和以及值域。用 [`src/battery.rs`](src/battery.rs) 表达实报百分比、粗档、估算、缓存或未知。
- 需要实验开关时，沿用 [`src/settings/providers.rs`](src/settings/providers.rs) 的策略。未通过对应实机验收的专用路径默认关闭，并记录精确适用范围。

## 4. 验证并提交结果

1. 为新匹配和解析补关键回归，覆盖合格与相邻不合格接口、截断或错误回复、连接丢失和取消后的迟到读数；注明输入来自真实报文脱敏还是人工构造。
2. 运行 `pwsh -NoProfile -File scripts/check.ps1`，再按改动范围运行 Release 构建及相关 UI、Agent 检查。
3. 在真实设备上用厂商软件或其他独立基准对照读数，记录来源、精度和测试条件。检查充电、断开重连、休眠唤醒、模式切换和厂商软件共存；无法复现的状态标为待测。
4. 在 PR 描述中列出已确认能力和未测项，附上脱敏命令、版本、退出码和结果。只有真实硬件读到了有效电量，才能声称该路径可用。

只提交设备反馈时，给出设备信息、预期与实际结果、操作顺序和脱敏输出即可。提交代码时再补协议来源、受影响型号、验证结果与未覆盖状态。
