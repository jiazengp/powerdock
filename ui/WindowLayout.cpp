#include "pch.h"
#include "Diagnostics.h"
#include "MainWindow.xaml.h"
#include <cstdio>
#include <winrt/Microsoft.UI.Xaml.Automation.Peers.h>
#include <winrt/Microsoft.UI.Xaml.Automation.Provider.h>
using namespace winrt;
using namespace Microsoft::UI::Xaml;
using namespace Microsoft::UI::Xaml::Controls;
using namespace Windows::Data::Json;
namespace winrt::PowerDockUI::implementation
{
bool MainWindow::ParentAlive() const
{
    DWORD process = 0;
    return host && IsWindow(host) && GetWindowThreadProcessId(host, &process) &&
           process == parentPid;
}
void MainWindow::ApplyFrameTheme() noexcept
{
    if (closed || !frameReady || !hwnd || !IsWindow(hwnd) || frameUpdating)
        return;
    frameUpdating = true;
    struct FrameScope
    {
        bool &flag;
        ~FrameScope()
        {
            flag = false;
        }
    } scope{frameUpdating};
    try
    {
        HIGHCONTRASTW contrast{sizeof(contrast)};
        const bool highContrast =
            !SystemParametersInfoW(SPI_GETHIGHCONTRAST, sizeof(contrast), &contrast, 0) ||
            (contrast.dwFlags & HCF_HIGHCONTRASTON) != 0;
        const auto corner=highContrast?DWMWCP_DEFAULT:DWMWCP_ROUND;
        DWM_WINDOW_CORNER_PREFERENCE currentCorner{};
        if(FAILED(DwmGetWindowAttribute(hwnd,DWMWA_WINDOW_CORNER_PREFERENCE,&currentCorner,sizeof(currentCorner))) || currentCorner!=corner)
            DwmSetWindowAttribute(hwnd,DWMWA_WINDOW_CORNER_PREFERENCE,&corner,sizeof(corner));
        const BOOL dark = !highContrast && Root().ActualTheme() == ElementTheme::Dark;
        const COLORREF requestedBorder = frameProbe && preview && frameMarker
            ? RGB(255,0,255) : highContrast ? DWMWA_COLOR_DEFAULT : DWMWA_COLOR_NONE;
        BOOL actualDark = FALSE;
        auto getDark = DwmGetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &actualDark,
                                             sizeof(actualDark));
        // Avoid setting an unchanged attribute, including on synchronous theme re-entry.
        const auto setDark =
            SUCCEEDED(getDark) && actualDark == dark
                ? S_FALSE
                : DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark, sizeof(dark));
        // BORDER_COLOR is a setter attribute; this Windows build rejects attempts to read it.
        const auto setBorder = DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, &requestedBorder,
                                                     sizeof(requestedBorder));
        getDark = DwmGetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &actualDark,
                                        sizeof(actualDark));
        if (preview || FAILED(setDark) || FAILED(setBorder) || FAILED(getDark))
        {
            char diagnostic[512]{};
            std::snprintf(diagnostic, sizeof(diagnostic),
                          "frame_theme dark=%d high_contrast=%d border_requested=0x%08lX "
                          "border_readback=unsupported dark_actual=%d set_dark=0x%08lX "
                          "set_border=0x%08lX get_dark=0x%08lX\n",
                          dark, highContrast, requestedBorder, actualDark,
                          static_cast<unsigned long>(setDark),
                          static_cast<unsigned long>(setBorder),
                          static_cast<unsigned long>(getDark));
            OutputDebugStringA(diagnostic);
            if (preview)
            {
                std::fputs(diagnostic, stdout);
                std::fflush(stdout);
            }
        }
        if (preview)
        {
            BOOL ncRendering=FALSE;
            UINT borderPixels=0, cloaked=0;
            RECT visibleFrame{};
            const auto ncStatus=DwmGetWindowAttribute(hwnd,DWMWA_NCRENDERING_ENABLED,&ncRendering,sizeof(ncRendering));
            const auto visibleStatus=DwmGetWindowAttribute(hwnd,DWMWA_EXTENDED_FRAME_BOUNDS,&visibleFrame,sizeof(visibleFrame));
            const auto borderStatus=DwmGetWindowAttribute(hwnd,DWMWA_VISIBLE_FRAME_BORDER_THICKNESS,&borderPixels,sizeof(borderPixels));
            DwmGetWindowAttribute(hwnd,DWMWA_CLOAKED,&cloaked,sizeof(cloaked));
            char probe[320]{};
            std::snprintf(probe,sizeof(probe),"frame_dwm nc=%d nc_hr=0x%08lX visible_hr=0x%08lX border_px=%u border_hr=0x%08lX corner=%d cloaked=%u visible=[%ld,%ld,%ld,%ld] dpi=%u\n",ncRendering,static_cast<unsigned long>(ncStatus),static_cast<unsigned long>(visibleStatus),borderPixels,static_cast<unsigned long>(borderStatus),static_cast<int>(currentCorner),cloaked,visibleFrame.left,visibleFrame.top,visibleFrame.right,visibleFrame.bottom,GetDpiForWindow(hwnd));
            std::fputs(probe,stdout);
            RECT window{}, client{};
            const BOOL rectangles = GetWindowRect(hwnd, &window) && GetClientRect(hwnd, &client);
            POINT origin{client.left, client.top}, end{client.right, client.bottom};
            const BOOL geometry =
                rectangles && ClientToScreen(hwnd, &origin) && ClientToScreen(hwnd, &end);
            const auto style = static_cast<unsigned long>(GetWindowLongPtrW(hwnd, GWL_STYLE));
            const auto extended = static_cast<unsigned long>(GetWindowLongPtrW(hwnd, GWL_EXSTYLE));
            char diagnostic[640]{};
            std::snprintf(
                diagnostic, sizeof(diagnostic),
                "frame_geometry valid=%d style=0x%08lX exstyle=0x%08lX WS_BORDER=%d WS_DLGFRAME=%d "
                "WS_THICKFRAME=%d window=[%ld,%ld,%ld,%ld] client=[%ld,%ld,%ld,%ld] "
                "client_screen=[%ld,%ld,%ld,%ld] insets=[%ld,%ld,%ld,%ld]\n",
                geometry, style, extended, (style & WS_BORDER) != 0, (style & WS_DLGFRAME) != 0,
                (style & WS_THICKFRAME) != 0, window.left, window.top, window.right, window.bottom,
                client.left, client.top, client.right, client.bottom, origin.x, origin.y, end.x,
                end.y, origin.x - window.left, origin.y - window.top, window.right - end.x,
                window.bottom - end.y);
            OutputDebugStringA(diagnostic);
            std::fputs(diagnostic, stdout);
            std::fflush(stdout);
        }
    }
    catch (...)
    {
        OutputDebugStringA("Unable to refresh native frame theme.\n");
    }
}
void MainWindow::PlaceWindow()
{
    POINT point{};
    GetCursorPos(&point);
    MONITORINFO monitor{sizeof(monitor)};
    const RECT placementAnchor =
        hasAnchor ? anchor : RECT{point.x, point.y, point.x + 1, point.y + 1};
    GetMonitorInfoW(MonitorFromRect(&placementAnchor, MONITOR_DEFAULTTONEAREST), &monitor);
    const double scale = GetDpiForWindow(hwnd) / 96.0;
    const LONG width = std::min<LONG>(
                   static_cast<LONG>((keepOpen ? 960 : 760) * scale),
                   std::max<LONG>(200, monitor.rcWork.right - monitor.rcWork.left - 16)),
               height = static_cast<LONG>(620 * scale);
    const LONG usableHeight = std::max<LONG>(200, monitor.rcWork.bottom - monitor.rcWork.top - 16);
    const LONG boundedHeight = std::min(height, usableHeight);
    LONG x = static_cast<LONG>(std::clamp<long long>(
        static_cast<long long>(placementAnchor.right) - width, monitor.rcWork.left,
        std::max(monitor.rcWork.left, monitor.rcWork.right - width)));
    LONG y = static_cast<LONG>(std::clamp<long long>(
        static_cast<long long>(placementAnchor.top) - boundedHeight - 12, monitor.rcWork.top,
        std::max(monitor.rcWork.top, monitor.rcWork.bottom - boundedHeight)));
    if (preview || keepOpen)
    {
        x = monitor.rcWork.left + (monitor.rcWork.right - monitor.rcWork.left - width) / 2;
        y = monitor.rcWork.top + (monitor.rcWork.bottom - monitor.rcWork.top - boundedHeight) / 2;
    }
    SetWindowPos(hwnd, nullptr, x, y, width, boundedHeight, SWP_NOZORDER | SWP_NOACTIVATE);
    placed = true;
    LayoutDevices();
    RefreshTitleBarLayout();
}
void MainWindow::LayoutDevices()
{
    const auto count = DeviceRows().Children().Size();
    const double available = DeviceRows().ActualWidth();
    const uint32_t columns =
        std::max(1u, std::min(count, static_cast<uint32_t>(std::max(1.0, (available + 12) / 272))));
    if (DeviceRows().ColumnDefinitions().Size() != columns ||
        DeviceRows().RowDefinitions().Size() != (count + columns - 1) / columns) {
        DeviceRows().ColumnDefinitions().Clear();
        for (uint32_t column = 0; column < columns; ++column)
            DeviceRows().ColumnDefinitions().Append(ColumnDefinition{});
        DeviceRows().RowDefinitions().Clear();
        for (uint32_t row = 0; row < (count + columns - 1) / columns; ++row)
        {
            RowDefinition definition;
            definition.Height(GridLength{1, GridUnitType::Auto});
            DeviceRows().RowDefinitions().Append(definition);
        }
    }
    for (uint32_t index = 0; index < count; ++index)
    {
        auto card = DeviceRows().Children().GetAt(index).as<FrameworkElement>();
        Grid::SetRow(card, index / columns);
        Grid::SetColumn(card, index % columns);
        Grid::SetColumnSpan(card, 1);
    }
}
void MainWindow::RefreshTitleBarLayout()
{
    if (!closed && AppTitleBar().XamlRoot()) AppTitleBar().RecomputeDragRegions();
}
}
