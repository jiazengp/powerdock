#include "pch.h"
#include "MainWindow.xaml.h"
#include <cmath>
#include <utility>

using namespace winrt;
using namespace Microsoft::UI::Xaml;
using namespace Microsoft::UI::Xaml::Controls;
using namespace Microsoft::UI::Windowing;

namespace winrt::PowerDockUI::implementation
{
void MainWindow::ActivateInitial()
{
    // A tray-only launch keeps the main window hidden. The host supplies the
    // localized state before requesting the menu through OpenPage.
    if (!menuOnly) Activate();
    else if (!host) ShowTrayMenu();
}

void MainWindow::CloseTrayMenu()
{
    // Detach before closing: deactivation and Closed can re-enter this method.
    // Keep the Window alive until its native Close call and event dispatch finish.
    auto menu = std::exchange(trayMenuWindow, nullptr);
    if (menu) DispatcherQueue().TryEnqueue([menu] { menu.Close(); });
}

void MainWindow::OpenFromTray(int page)
{
    menuOnly = false;
    NavigatePage(page);
    Activate();
    ShowWindow(hwnd, SW_SHOWNORMAL);
    SetForegroundWindow(hwnd);
    CloseTrayMenu();
}

void MainWindow::ShowTrayMenu()
{
    if (closed) return;
    if (trayMenuWindow) { trayMenuWindow.Activate(); return; }

    Window menuWindow;
    trayMenuWindow = menuWindow;
    menuWindow.Title(Text(L"AppName"));
    auto presenter = OverlappedPresenter::CreateForContextMenu();
    presenter.SetBorderAndTitleBar(false, false);
    menuWindow.AppWindow().SetPresenter(presenter);
    menuWindow.AppWindow().IsShownInSwitchers(false);
    menuWindow.SystemBackdrop(Media::DesktopAcrylicBackdrop{});

    MenuFlyoutPresenter menu;
    menu.RequestedTheme(Root().ActualTheme());
    menu.Language(locale);
    menu.MinWidth(220);
    menu.MaxWidth(360);
    auto append = [&](hstring const &key, hstring const &glyph, int page) {
        MenuFlyoutItem item;
        item.Text(Text(key));
        FontIcon icon;
        icon.Glyph(glyph);
        item.Icon(icon);
        item.Click([weak=get_weak(),page](auto const &,auto const &) {
            if (auto self=weak.get()) {
                // Do not tear down the menu while its item is dispatching Click.
                self->DispatcherQueue().TryEnqueue([weak,page] {
                    if(auto current=weak.get();current && !current->closed) {
                        if(page < 0) {
                            current->SendCommand(103);
                            current->CloseTrayMenu();
                        } else current->OpenFromTray(page);
                    }
                });
            }
        });
        menu.Items().Append(item);
    };
    append(L"Overview",L"\uE975",0);
    append(L"Settings",L"\uE713",1);
    menu.Items().Append(MenuFlyoutSeparator{});
    append(L"Exit",L"\uE7E8",-1);
    menu.KeyDown([weak=get_weak()](auto const &,Input::KeyRoutedEventArgs const &e) {
        if (e.Key()==Windows::System::VirtualKey::Escape) {
            e.Handled(true);
            if (auto self=weak.get()) self->CloseTrayMenu();
        }
    });
    menuWindow.Content(menu);
    HWND menuHwnd{};
    menuWindow.as<IWindowNative>()->get_WindowHandle(&menuHwnd);
    auto style=GetWindowLongPtrW(menuHwnd,GWL_EXSTYLE);
    SetWindowLongPtrW(menuHwnd,GWL_EXSTYLE,(style|WS_EX_TOOLWINDOW)&~WS_EX_APPWINDOW);
    int corner=DWMWCP_ROUND;
    DwmSetWindowAttribute(menuHwnd,DWMWA_WINDOW_CORNER_PREFERENCE,&corner,sizeof(corner));

    // Measure the real native menu (including Windows text scaling), then fit it
    // to the taskbar monitor work area, including left/right or auto-hide taskbars.
    menu.Measure({360,1000});
    auto desired=menu.DesiredSize();
    const auto scale=GetDpiForWindow(menuHwnd)/96.0;
    LONG width=static_cast<LONG>(std::ceil(std::max(220.0f,desired.Width)*scale));
    LONG height=static_cast<LONG>(std::ceil(desired.Height*scale));
    POINT cursor{};
    GetCursorPos(&cursor);
    MONITORINFO monitor{sizeof(monitor)};
    GetMonitorInfoW(MonitorFromPoint(cursor,MONITOR_DEFAULTTONEAREST),&monitor);
    width=std::min(width,monitor.rcWork.right-monitor.rcWork.left);
    height=std::min(height,monitor.rcWork.bottom-monitor.rcWork.top);
    const LONG x=std::clamp(cursor.x-width,monitor.rcWork.left,monitor.rcWork.right-width);
    const LONG y=std::clamp(cursor.y-height,monitor.rcWork.top,monitor.rcWork.bottom-height);
    menuWindow.AppWindow().ResizeClient({width,height});
    menuWindow.AppWindow().Move({x,y});

    menuWindow.Closed([weak=get_weak()](auto const &sender,auto const &) {
        if (auto self=weak.get()) {
            if(self->trayMenuWindow == sender) self->trayMenuWindow=nullptr;
            // Finish this window's event dispatch before closing the hidden owner.
            self->DispatcherQueue().TryEnqueue([weak] {
                if(auto current=weak.get();current && current->menuOnly && !current->closed && !current->trayMenuWindow)
                    current->Close();
            });
        }
    });
    menuWindow.Activated([weak=get_weak()](auto const &,WindowActivatedEventArgs const &e) {
        if (e.WindowActivationState()==WindowActivationState::Deactivated)
            if (auto self=weak.get();self && !self->smokeTest && !self->smokeNavigation) self->CloseTrayMenu();
    });
    menuWindow.Activate();
    SetForegroundWindow(menuHwnd);
    menu.Items().GetAt(0).as<MenuFlyoutItem>().Focus(FocusState::Keyboard);
}
}
