# Template and source attribution

Structured local diagnostics use Tokio tracing 0.1.44, tracing-subscriber 0.3.23 and tracing-appender 0.2.5 (MIT), with existing regex and zip dependencies for redaction and bounded local export. Their original license notices are collected from locked Cargo package sources into vendor/dependency-licenses/. komorebi and Starward were inspected for operational patterns; no komorebi project code or Starward logging/upload code is included. The implementation is contained in the diagnostics modules.

The notification window's transparent system-backdrop brush and DWM background-clearing pattern are adapted to C++ from dotMorten/WinUIEx, commit `5897280a62f954153d5e8ed578eb8f250382e970`, `src/WinUIEx/TransparentTintBackdrop.cs` and `CompositionBrushBackdrop.cs` (MIT, copyright Morten Nielsen). The notice is retained in `vendor/WINUIEX-MIT.txt`. No WinUIEx managed assembly or .NET runtime is included. The layered card entrance/exit sequence is informed by OBSNotifier's NVIDIA-like module; its WPF animation code and artwork are not distributed.

The native window lifecycle and message-loop starting point is Microsoft windows-rs's create-window example:

- Repository: https://github.com/microsoft/windows-rs
- Snapshot: `0cfb708054687e5bc32c596a98cfcf1395344eab` (2026-11-09)
- Path: `crates/samples/windows-sys/create-window/src/main.rs`
- License: MIT OR Apache-2.0; the MIT option is used. The original reference and Microsoft MIT notice are retained in `vendor/`.

The current upstream sample uses a newer binding layout; this project adapts its window/message-loop structure to stable windows-sys 0.61.2. It does not include a GUI framework or WebView runtime.

Other investigated starting points: tauri-apps/tray-icon (Apache-2.0), mature tray lifecycle but an additional event-loop abstraction was unnecessary here; aliezzzz/mchose-battery (MIT), not verified for A7 V3 Pro; Tensyd/mchose-battery, no explicit license found and not used as code.

The A7 V3 Pro packet format was independently implemented from the vendor's public driver behavior and verified against a real receiver. No MCHOSE application, firmware, JavaScript bundle or branding assets are distributed with this project.

Cargo.lock records the exact dependency versions; each dependency retains its upstream license.

The modern C++/WinUI frontend is adapted from Microsoft WindowsAppSDK-Samples, pinned to f1be017f26a0feae4c8dc805748eab0a1f7b4fc6. Its MIT notice is retained as ui/UPSTREAM-LICENSE.txt; see ui/SOURCE.md. NuGet transitive versions are in ui/packages.lock.json. The build script copies provided package redistribution notices into the runtime licenses/ directory.

Rust dependency license notices are retained under vendor/dependency-licenses/. The tray uses the documented Mouse glyph U+E962 from Windows 11 installed Segoe Fluent Icons, . Font files are not copied or redistributed. Dynamic level coloring and composition are this project's code.

The v0.2 system battery provider references the MIT-licensed HaloBattery provider architecture and Bluetooth/PnP metadata approach at commit 2251f0534a8e886c9692ead42056059b4f69d8c0. The original MIT notice is retained in vendor/HALOBATTERY-MIT.txt. The Rust implementation uses the existing Microsoft windows-rs bindings directly. No Solaar GPL implementation or unlicensed candidate code is included.

Agent CLI argument parsing uses clap 4.6.7 (MIT OR Apache-2.0, MIT option) and its locked dependencies. Redistribution notices for clap, clap_builder, clap_derive, clap_lex, anstyle, heck and strsim are retained under vendor/dependency-licenses/. No MCP runtime or local-socket server is included.

The 41E4:2001 keyboard battery path references public packet captures in HaloBattery at 73e229cbee2e97fad659fcd96c78afdfa78488fb (MIT; notice already retained). Public numerical protocol facts were cross-checked against kuz-1/mchosebar fc18f6952ce167fda6a6aea60af6fd8d47223194 (GPL-3.0); no GPL implementation or assets were copied.

The MOMENTUM 4 read-only GAIA protocol was independently implemented using packet captures and framing specifications from Ariazonaa/momentum-control 1694dc48623b507328c8d28739d7a2e01326b505 (MIT). The original Copyright (c) 2026 Ariazonaa notice and full license are retained in vendor/MOMENTUM-CONTROL-MIT.txt. Windows container metadata lookup references Ariazonaa/PeripheralBatteryPlugin 8b90b8289ce97bb99927467eea44a934d0f755f9 (MIT) as an API usage pattern, without copied implementation. Direct windows-collections 0.3.2 uses the already redistributed Microsoft MIT notice.

Standard BLE Battery Service and paired endpoint discovery use Microsoft Windows SDK contracts and official Windows-universal-samples API usage examples, independently implemented in Rust. Direct windows-future 0.3.2 is already part of windows-rs and covered by the retained Microsoft MIT dependency notice; no extra async runtime or third-party protocol daemon is shipped. Public MCHOSE model-table numerical facts identify the A7 V3 Pro USB 4031 and receiver 1014/1018 interfaces; no official application code or firmware is redistributed.


v0.3 uses Velopack 1.3.39-g8d2844f (MIT) for native delta/installer support, zip 9.8.2 (MIT) for local diagnostic archives, rust-i18n 4.2.4 (MIT) for compiled localization and RustCrypto sha2 0.11 (MIT OR Apache-2.0) for installation digest checks. Original notices and locked dependency notices are retained in vendor/dependency-licenses/. The separately pinned vpk build tool does not introduce a .NET application runtime.

Starward (Scighost, MIT, df8ae07b5ad4c9c6cb70d288488ce8073ce01478) informed General/About information structure and update states. No Starward code, artwork or private Windows hooks are distributed. New native caption and MRT handling use public Microsoft APIs independently.

## Fluent System Icons application icon

The PowerDock application tile incorporates Microsoft's Battery Charge 24 Filled SVG from `microsoft/fluentui-system-icons`, commit `67e6dec1ee956af3ab4c746a2b3bb6a2e5520a44`, recolored on a PowerDock blue tile. The cards also use the Flash 16 Filled SVG geometry from the same commit for the solid charging bolt. Original SVGs and source links are retained under assets/. The MIT license is retained in `assets/FLUENT-ICONS-LICENSE.txt` and packaged UI licenses.

`winresource` (MIT) is a build-only resource compiler helper. Python resvg/Pillow are used only to regenerate committed icon assets, and are not part of the installed application runtime.

## HID++ protocol library

PowerDock links the published `openlogi-hidpp =0.9.0` protocol crate under 0BSD and uses `openlogi-device-registry =0.9.0` for the published Logitech receiver classification (MIT OR Apache-2.0, MIT option). Its derive macro package has the same dual license; original notices are retained. The protocol crate archive checksum is recorded in Cargo.lock, VCS source `a5b507ec8802e0ecb7ef3de61ce7868609c9b533`, repository https://github.com/AprilNEA/OpenLogi. The application repository's Apache-2.0 license is not substituted for the separately licensed protocol package. Exact transitive notices are collected under vendor/dependency-licenses/.

The local Windows transport uses Microsoft WinRT HID APIs and bounded async-channel/futures primitives. No OpenLogi GUI/core/Agent, Tokio runtime, upstream native SET/feature fallback, or `openlogi-async-hid` code is included. Futures, futures-timer and async-channel retain their original locked package licenses. Protocol support remains experimental until Windows hardware acceptance; library decoder tests do not establish model compatibility.

The experimental MCHOSE Sinowealth wireless keyboard fingerprint and read-only battery exchange were checked against OpenMHub f77c000334ad9e34499d7acab315a0d9ee8789b7 (MIT), app/renderer/drivers/sinowealth.js. It reuses the existing native transport and parser; no settings/configuration initialization was copied. Copyright (c) 2026 Brayan Spagnol and the MIT text are retained in vendor/OPENMHUB-MIT.txt.

Sennheiser product/capability and typed reply decoding reuse the unmodified pure protocol crate from xndbogdan/sennheiser-control, fixed commit 5ec3c873fb8f6b9d1d7225edec8cf566f1a485d6, MIT as declared in the preserved upstream manifests. See vendor/sennheiser-protocol/NOTICE.md and PROVENANCE.json. No GUI, engine, Bluetooth scanner, configuration initialization or runtime settings request is invoked; the local send allowlist is four identity/battery GETs.

The experimental MCHOSE YJX mouse battery/status exchange and exact model/transport table are adapted from OpenMHub f77c000334ad9e34499d7acab315a0d9ee8789b7, app/renderer/drivers/g3v2.js and base.js, MIT. Windows report-ID padding follows the public HIDP_CAPS contract. Existing native transport is reused; no configuration initialization, firmware, DPI, keys or lighting commands are exposed. Copyright (c) 2026 Brayan Spagnol and full MIT notice remain in vendor/OPENMHUB-MIT.txt.


The experimental SteelSeries direct-percent battery path references SteelMouse (yurtemre7/steel-mouse), pinned to d6a849bbf5b07357a7d25095d224b60f0076db85 (MIT). The original Emre Yurtseven license is retained in vendor/STEELMOUSE-MIT.txt. Only the three reviewed direct-percent PID/command facts and Windows response framing are adapted; no rivalcfg database, GUI, analyzer, generic command probing, or alternate feature-write fallback is included. Hardware/charging validation remains pending.
