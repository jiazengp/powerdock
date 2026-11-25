# Native frontend source

The WinUI application/project lifecycle is adapted from the MIT-licensed Microsoft WindowsAppSDK-Samples self-contained unpackaged C++ sample, pinned to f1be017f26a0feae4c8dc805748eab0a1f7b4fc6:
https://github.com/microsoft/WindowsAppSDK-Samples/tree/f1be017f26a0feae4c8dc805748eab0a1f7b4fc6/Samples/SelfContainedDeployment/cpp/cpp-winui-unpackaged

Its original license is retained in UPSTREAM-LICENSE.txt. Windows App SDK redistribution remains subject to its package terms; build-ui.ps1 copies supplied notices into licenses/. Exact package versions are locked in packages.lock.json.

The frontend uses Windows App SDK WinUI 2.6.5, C++/WinRT 2.0.240405.15, Visual Studio 2026 x64 tools and Windows SDK 26100. The win10-x64 NuGet RID is a tooling identifier; the application targets Windows 11 build 26100 or newer. The split WinUI package avoids unused AI, ML, Widgets and Search components. No WebView or .NET application runtime is needed.

v0.3 uses the public TitleBar control, extended content, native minimize/maximize/restore/close, native back, a compact left NavigationView, public AppWindow theme APIs and Mica. The resizable presenter keeps the system frame; DWM border color is suppressed outside high contrast. The previous unconditional WM_NCCALCSIZE workaround, WS_DLGFRAME stripping and manual caption/resize rectangles have been removed. MenuFlyout is a real themed WinUI control and is placed below icon buttons. No private uxtheme ordinal or InputSiteWindowClass patch is used.

Local MRT resource contexts provide immediate system/Chinese/English switching from the shared JSON catalog. Controls use ThemeResource brushes and native focus/keyboard roles. UI preview accepts --theme light/dark without changing Windows settings. The real host supports a persisted system/light/dark preference.

The frontend starts on demand and exits on close; minimizing keeps its native taskbar window. Rust monitoring continues independently. --keep-open selects a wider inspection layout while retaining real host state and parent-process checks. --preview is an isolated simulated UI and rejects real mutations.

Official guidance: https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/title-bar


The title bar displays no application title text; the window retains its accessible app name for task switching. The fixed compact left sidebar contains Devices and Settings. Settings contains General, Notifications and About tabs. There is no pane-toggle, About or Quit action in the sidebar. Refresh/sort remain beside the device heading. Native caption buttons supply Snap Layout and system accessibility behavior.


The 48-DIP compact navigation rail spans the entire window height. The 48-DIP native caption occupies the content area to its right, with Tall native caption buttons. Content spacing follows the actual TitleBar height. NavigationView content uses a transparent layer instead of an opaque gray fill; its extra content border is removed.
