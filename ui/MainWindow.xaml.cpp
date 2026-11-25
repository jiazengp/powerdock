#include "pch.h"
#include "Diagnostics.h"
#include "MainWindow.xaml.h"
#include <cerrno>
#include <cwchar>
#include <cstdio>
#include <winrt/Microsoft.UI.Xaml.Automation.Peers.h>
#include <winrt/Microsoft.UI.Xaml.Automation.Provider.h>
#if __has_include("MainWindow.g.cpp")
#include "MainWindow.g.cpp"
#endif
using namespace winrt;
using namespace Microsoft::UI::Xaml;
using namespace Microsoft::UI::Xaml::Controls;
using namespace Windows::Data::Json;

namespace winrt::PowerDockUI::implementation
{
namespace
{
constexpr UINT RegisterUi = WM_APP + 7, UnregisterUi = WM_APP + 8, OpenPage = WM_APP + 9;
}
MainWindow::MainWindow()
{
    InitializeComponent();
    int count = 0;
    ElementTheme previewTheme = ElementTheme::Default;
    auto arguments = CommandLineToArgvW(GetCommandLineW(), &count);
    for (int index = 1; arguments && index < count; ++index)
    {
        std::wstring_view option(arguments[index]);
        if (option == L"--frame-probe")
            frameProbe = true;
        else if (option == L"--smoke-charging")
            smokeCharging = true;
        else if (option == L"--smoke-navigation")
            smokeNavigation = true;
        else if (option == L"--smoke-test")
            smokeTest = true;
        else if (option == L"--preview")
            preview = true;
        else if (option == L"--keep-open")
            keepOpen = true;
        else if (option == L"--host" && index + 1 < count)
            host = reinterpret_cast<HWND>(_wcstoui64(arguments[++index], nullptr, 10));
        else if (option == L"--parent" && index + 1 < count)
            parentPid = static_cast<DWORD>(_wcstoui64(arguments[++index], nullptr, 10));
        else if (option == L"--page" && index + 1 < count)
        {
            const std::wstring_view page(arguments[++index]);
            settingsPage = page == L"settings";
            menuOnly = page == L"tray";
        }
        else if (option == L"--anchor" && index + 4 < count)
        {
            LONG values[4]{};
            bool valid = true;
            for (int offset = 0; offset < 4; ++offset)
            {
                wchar_t *end = nullptr;
                const auto text = arguments[index + 1 + offset];
                errno = 0;
                const long value = std::wcstol(text, &end, 10);
                if (errno == ERANGE || end == text || !end || *end != L'\0')
                    valid = false;
                values[offset] = value;
            }
            index += 4;
            if (valid && values[2] > values[0] && values[3] > values[1])
            {
                anchor = RECT{values[0], values[1], values[2], values[3]};
                hasAnchor = true;
            }
        }
        else if (option == L"--theme" && index + 1 < count)
        {
            std::wstring_view theme(arguments[++index]);
            if (theme == L"light")
                previewTheme = ElementTheme::Light;
            else if (theme == L"dark")
                previewTheme = ElementTheme::Dark;
        }
    }
    if (arguments)
        LocalFree(arguments);
    if (!host)
        preview = true;
    Window window = *this;
    window.as<IWindowNative>()->get_WindowHandle(&hwnd);
    AppWindow().TitleBar().PreferredTheme(
        Microsoft::UI::Windowing::TitleBarTheme::UseDefaultAppMode);
    // The override exists only for isolated preview verification; production follows Windows.
    if (preview && previewTheme != ElementTheme::Default)
    {
        Root().RequestedTheme(previewTheme);
        AppWindow().TitleBar().PreferredTheme(previewTheme == ElementTheme::Dark
                                                  ? Microsoft::UI::Windowing::TitleBarTheme::Dark
                                                  : Microsoft::UI::Windowing::TitleBarTheme::Light);
    }
    SetWindowSubclass(hwnd, Subclass, 1, reinterpret_cast<DWORD_PTR>(this));
    // Configure the existing presenter before extending XAML into the title bar.
    // Replacing it afterwards can restore non-client frame defaults.
    auto presenter=AppWindow().Presenter().as<Microsoft::UI::Windowing::OverlappedPresenter>();
    presenter.IsResizable(true);
    presenter.IsMinimizable(true);
    presenter.IsMaximizable(true);
    ExtendsContentIntoTitleBar(true);
    SetTitleBar(AppTitleBar());
    AppWindow().TitleBar().PreferredHeightOption(Microsoft::UI::Windowing::TitleBarHeightOption::Tall);
    resourceManager=Microsoft::Windows::ApplicationModel::Resources::ResourceManager{
        Microsoft::Windows::ApplicationModel::Resources::ResourceLoader::GetDefaultResourceFilePath()};
    resources=resourceManager.MainResourceMap().GetSubtree(L"Resources");
    resourceContext=resourceManager.CreateResourceContext();
    locale=(GetUserDefaultUILanguage()&0x3ff)==4?L"zh-CN":L"en-US";
    resourceContext.QualifierValues().Insert(L"Language",locale);
    ApplyLabels();
    SettingsPage().SelectedItem(GeneralNav());
    Root().SizeChanged([weak = get_weak()](auto const &, auto const &) {
        if (auto self = weak.get(); self && !self->closed)
        {
            self->LayoutDevices();
            self->RefreshTitleBarLayout();
        }
    });
    Root().Loaded([weak=get_weak()](auto const &,auto const &) {
        if(auto self=weak.get()) self->DispatcherQueue().TryEnqueue([weak] {
            if(auto current=weak.get();current && !current->closed)current->ApplyFrameTheme();
        });
    });
    DeviceRows().SizeChanged([weak=get_weak()](auto const &,auto const &){if(auto self=weak.get();self && !self->closed)self->LayoutDevices();});
    const auto style=GetWindowLongPtrW(hwnd,GWL_EXSTYLE);
    SetWindowLongPtrW(hwnd,GWL_EXSTYLE,(style|WS_EX_APPWINDOW)&~WS_EX_TOOLWINDOW);
    wchar_t modulePath[32768]{};
    GetModuleFileNameW(nullptr,modulePath,32768);
    const auto iconPath=std::filesystem::path{modulePath}.parent_path()/L"Assets"/L"PowerDock.ico";
    AppWindow().SetTitleBarIcon(iconPath.wstring());
    AppWindow().SetTaskbarIcon(iconPath.wstring());
    int corner = DWMWCP_ROUND;
    DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &corner, sizeof(corner));
    Root().ActualThemeChanged([weak = get_weak()](auto const &, auto const &) {
        if (auto self = weak.get())
            self->ApplyFrameTheme();
    });
    frameReady = true;
    ApplyFrameTheme();
    PlaceWindow();
    ShowPage(settingsPage);
    ThresholdChoice().SelectedIndex(2);
    IntervalChoice().SelectedIndex(1);
    updating = false;
    if(frameProbe && preview) {
        Button probe;probe.Name(L"FrameProbeToggle");probe.Content(box_value(L"系统描边：品红色"));
        probe.HorizontalAlignment(HorizontalAlignment::Right);probe.VerticalAlignment(VerticalAlignment::Top);probe.Margin({24,0,24,0});
        Grid::SetRow(probe,1);
        probe.Click([weak=get_weak()](Windows::Foundation::IInspectable const &sender,RoutedEventArgs const &) {
            if(auto self=weak.get()) {self->frameMarker=!self->frameMarker;self->ApplyFrameTheme();
                sender.as<Button>().Content(box_value(self->frameMarker?L"系统描边：品红色":L"系统描边：隐藏"));}
        });
        Root().Children().Append(probe);Title(L"PowerDock — frame diagnostic");
    }
    auto accelerator=[this](Windows::System::VirtualKey key,Windows::System::VirtualKeyModifiers modifiers,auto action){
        Input::KeyboardAccelerator value;value.Key(key);value.Modifiers(modifiers);
        value.Invoked([weak=get_weak(),action](auto const &,Input::KeyboardAcceleratorInvokedEventArgs const &e){if(auto self=weak.get()){action(*self);e.Handled(true);}});
        Root().KeyboardAccelerators().Append(value);
    };
    accelerator(Windows::System::VirtualKey::F5,Windows::System::VirtualKeyModifiers::None,[](auto &self){self.SendCommand(101);});
    accelerator(Windows::System::VirtualKey::F1,Windows::System::VirtualKeyModifiers::None,[](auto &self){self.NavigatePage(2);});
    accelerator(static_cast<Windows::System::VirtualKey>(VK_OEM_COMMA),Windows::System::VirtualKeyModifiers::Control,[](auto &self){self.SettingsPage().SelectedItem(self.GeneralNav());self.ShowPage(true);});
    Root().KeyDown([weak = get_weak()](auto const &, Input::KeyRoutedEventArgs const &e) {
        if (auto self = weak.get(); self && e.Key() == Windows::System::VirtualKey::Escape)
        {
            e.Handled(true);
            self->Close();
        }
    });
    Closed([weak = get_weak()](auto const &, auto const &) {
        if (auto self = weak.get())
        {
            if(self->overlayTransparencyTimer)self->overlayTransparencyTimer.Stop();
            if(self->diagnosticsFeedbackTimer)self->diagnosticsFeedbackTimer.Stop();
            if(self->overlayTransparencyPending) {
                self->overlayTransparencyPending=false;
                self->PatchPreference(L"overlay_transparency",JsonValue::CreateNumberValue(self->overlayTransparencyValue));
            }
            self->closed = true;
            if (self->timer)
                self->timer.Stop();
            if (self->host)
                PostMessageW(self->host, UnregisterUi, reinterpret_cast<WPARAM>(self->hwnd), 0);
            Application::Current().Exit();
        }
    });
    timer = DispatcherTimer();
    timer.Interval(std::chrono::milliseconds(500));
    timer.Tick([weak = get_weak()](auto const &, auto const &) {
        if (auto self = weak.get(); self && !self->closed)
        {
            if(self->smokeCharging) {self->AdvanceChargingSmoke();return;}
            if(self->smokeNavigation) {self->AdvanceNavigationSmoke();return;}
            if(self->smokeTest && ++self->smokeTicks>=4){std::fputs("WinUI startup passed\n",stdout);std::fflush(stdout);self->Close();return;}
            if(self->chargingFeedbackActive)self->RefreshChargingPresentation();
            if (self->host && !self->ParentAlive())
            {
                self->Close();
                return;
            }

        }
    });
    timer.Start();
    Activated([weak = get_weak()](auto const &, WindowActivatedEventArgs const &e) {
        if (auto self = weak.get();
            self && e.WindowActivationState() != WindowActivationState::Deactivated)
        {
            self->hasActivated = true;
            self->ApplyFrameTheme();
            if (self->host && self->ParentAlive())
                PostMessageW(self->host, RegisterUi, reinterpret_cast<WPARAM>(self->hwnd), 0);
        }
    });
    if (host && ParentAlive())
        PostMessageW(host, RegisterUi, reinterpret_cast<WPARAM>(hwnd), 0);
}
MainWindow::~MainWindow()
{
    if (hwnd)
        RemoveWindowSubclass(hwnd, Subclass, 1);
}
LRESULT CALLBACK MainWindow::Subclass(HWND window, UINT message, WPARAM wp, LPARAM lp, UINT_PTR,
                                      DWORD_PTR reference)
{
    auto self = reinterpret_cast<MainWindow *>(reference);
    if (message == WM_NCDESTROY)
    {
        RemoveWindowSubclass(window, Subclass, 1);
        self->hwnd = nullptr;
        return DefSubclassProc(window, message, wp, lp);
    }
    if (self->closed && (message == WM_COPYDATA || message == OpenPage))
        return FALSE;
    if ((message == WM_STYLECHANGED && static_cast<int>(wp) == GWL_STYLE) || message == WM_SIZE)
    {
        auto lifetime = self->get_strong();
        const auto result = DefSubclassProc(window, message, wp, lp);
        lifetime->ApplyFrameTheme();
        if (message == WM_SIZE && lifetime->frameReady && !lifetime->closed)
        {
            lifetime->LayoutDevices();
            lifetime->RefreshTitleBarLayout();
        }
        return result;
    }
    if (message == WM_COPYDATA && self->host && wp == reinterpret_cast<WPARAM>(self->host) &&
        self->ParentAlive())
    {
        auto data = reinterpret_cast<COPYDATASTRUCT const *>(lp);
        if (data && data->lpData)
        {
            try
            {
                return self->ReceiveStatePacket(
                    data->dwData, {static_cast<char const *>(data->lpData), data->cbData});
            }
            catch (hresult_error const &error)
            {
                self->pendingState.clear();
                OutputDebugStringW(error.message().c_str());
                return FALSE;
            }
            catch (...)
            {
                self->pendingState.clear();
                OutputDebugStringW(L"Rejected frontend state at the Win32 boundary.");
                return FALSE;
            }
        }
        return FALSE;
    }
    if (message == OpenPage)
    {
        if(wp == 2) self->ShowTrayMenu();
        else self->OpenFromTray(wp == 0 ? 0 : 1);
        return 0;
    }
    if (message == WM_GETMINMAXINFO && lp)
    {
        const auto result=DefSubclassProc(window,message,wp,lp);
        auto info = reinterpret_cast<MINMAXINFO *>(lp);
        const double scale = GetDpiForWindow(window) / 96.0;
        info->ptMinTrackSize =
            POINT{static_cast<LONG>(360 * scale), static_cast<LONG>(400 * scale)};
        return result;
    }
    if (message == WM_SETTINGCHANGE || message == WM_THEMECHANGED || message == WM_SYSCOLORCHANGE)
        self->ApplyFrameTheme();
    return DefSubclassProc(window, message, wp, lp);
}
void MainWindow::RefreshClick(IInspectable const &, RoutedEventArgs const &)
{
    SendCommand(101);
}
void MainWindow::SettingsClick(IInspectable const &, RoutedEventArgs const &)
{
    NavigatePage(1);
}
void MainWindow::BackClick(IInspectable const &, RoutedEventArgs const &)
{
    NavigatePage(0);
}



void MainWindow::TestNotificationClick(IInspectable const &, RoutedEventArgs const &)
{
    SendCommand(105);
}

void MainWindow::ThresholdChanged(IInspectable const &, SelectionChangedEventArgs const &)
{
    if (!updating)
    {
        constexpr WORD ids[]{200, 210, 220, 230};
        int selected = ThresholdChoice().SelectedIndex();
        if (selected >= 0 && selected < 4)
            SendCommand(ids[selected]);
    }
}
void MainWindow::IntervalChanged(IInspectable const &, SelectionChangedEventArgs const &)
{
    if (!updating)
    {
        constexpr WORD ids[]{330, 360, 420};
        int selected = IntervalChoice().SelectedIndex();
        if (selected >= 0 && selected < 3)
            SendCommand(ids[selected]);
    }
}
void MainWindow::StartupChanged(IInspectable const &, RoutedEventArgs const &)
{
    if (!updating)
        SendCommand(104);
}
void MainWindow::SortClick(IInspectable const &sender, RoutedEventArgs const &)
{
    if (!updating)
    {
        const auto tag = unbox_value<hstring>(sender.as<RadioMenuFlyoutItem>().Tag());
        SendCommand(static_cast<WORD>(std::wcstoul(tag.c_str(), nullptr, 10)));
    }
}
} // namespace winrt::PowerDockUI::implementation
