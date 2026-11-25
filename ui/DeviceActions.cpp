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
void MainWindow::RefreshDevice(hstring const &id)
{
    if (preview || !ParentAlive())
        return;
    auto bytes = to_string(id);
    COPYDATASTRUCT data{2, static_cast<DWORD>(bytes.size()), bytes.data()};
    DWORD_PTR result = 0;
    SendMessageTimeoutW(host, WM_COPYDATA, reinterpret_cast<WPARAM>(hwnd),
                        reinterpret_cast<LPARAM>(&data), SMTO_ABORTIFHUNG, 200,
                        &result);
}
void MainWindow::ShowPage(bool settings) { NavigatePage(settings?1:0); }
void MainWindow::NavigatePage(int page)
{
    PowerDockTrace(L"page_navigated",Windows::Data::Json::JsonObject::Parse(L"{\"page\":"+to_hstring(page)+L"}"));
    const bool changed = settingsPage != (page != 0);
    settingsPage=page!=0;
    BatteryPage().Visibility(settingsPage?Visibility::Collapsed:Visibility::Visible);
    SettingsPage().Visibility(settingsPage?Visibility::Visible:Visibility::Collapsed);
    RefreshWindowTitle();
    if(page==2)SettingsPage().SelectedItem(AboutNav());
    else if(settingsPage && !SettingsPage().SelectedItem())SettingsPage().SelectedItem(GeneralNav());
    if(changed) AnimatePage(settingsPage ? SettingsPage().as<FrameworkElement>() : BatteryPage().as<FrameworkElement>(), 0);
    if(!placed)PlaceWindow();
    if(hasActivated) {
        if(settingsPage) SettingsPage().Focus(FocusState::Programmatic);
        else RefreshButton().Focus(FocusState::Programmatic);
    }
}
void MainWindow::SetDeviceReminder(hstring const &id, int threshold)
{
    if (preview || !ParentAlive())
        return;
    JsonObject operation;
    operation.Insert(L"operation", JsonValue::CreateStringValue(L"device_reminder_set"));
    operation.Insert(L"id", JsonValue::CreateStringValue(id));
    operation.Insert(L"threshold", threshold < 0 ? JsonValue::CreateNullValue()
                                                : JsonValue::CreateNumberValue(threshold));
    auto bytes = to_string(operation.Stringify());
    COPYDATASTRUCT data{3, static_cast<DWORD>(bytes.size()), bytes.data()};
    DWORD_PTR result = 0;
    // Permit synchronous state replies while the host saves and acknowledges this change.
    SendMessageTimeoutW(host, WM_COPYDATA, reinterpret_cast<WPARAM>(hwnd),
                        reinterpret_cast<LPARAM>(&data), SMTO_ABORTIFHUNG, 1500, &result);
}
MenuFlyoutSubItem MainWindow::CreateReminderMenu(hstring const &id)
{
    MenuFlyoutSubItem menu;
    menu.Name(L"DeviceReminderMenu");
    menu.Text(Text(L"DeviceReminder"));
    menu.IsEnabled(!preview);
    FontIcon icon;
    icon.Glyph(L"\uE7F4");
    menu.Icon(icon);
    for (const int threshold : {-1, 0, 10, 20, 30})
    {
        RadioMenuFlyoutItem choice;
        choice.GroupName(L"DeviceReminder-" + id);
        choice.Tag(box_value(threshold));
        choice.Text(threshold < 0 ? Format(L"FollowGlobal",{{L"value",Text(L"DefaultReminder")}})
                    : threshold == 0 ? Text(L"Off") : Format(L"BelowThreshold",{{L"value",to_hstring(threshold)}}));
        choice.Click([weak = get_weak(), id, threshold](auto const &, auto const &) {
            if (auto self = weak.get())
                self->SetDeviceReminder(id, threshold);
        });
        menu.Items().Append(choice);
    }
    MenuFlyoutItem note;
    note.Name(L"DeviceReminderNote");
    note.IsEnabled(false);
    note.Visibility(Visibility::Collapsed);
    menu.Items().Append(note);
    return menu;
}
void MainWindow::UpdateReminderMenu(FrameworkElement const &card, JsonObject const &device,
                                    int globalThreshold)
{
    const auto reminder = device.GetNamedObject(L"reminder", JsonObject{});
    const auto value = reminder.GetNamedValue(L"threshold", JsonValue::CreateNullValue());
    const int threshold = value.ValueType() == JsonValueType::Number
                              ? static_cast<int>(value.GetNumber()) : -1;
    const int effective = static_cast<int>(reminder.GetNamedNumber(L"effective_threshold", globalThreshold));
    const auto note = reminder.GetNamedString(L"note", L"");
    const auto flyout = card.FindName(L"CardMore").as<Button>().Flyout().as<MenuFlyout>();
    for (auto const &item : flyout.Items())
    {
        auto menu = item.try_as<MenuFlyoutSubItem>();
        if (!menu || menu.Name() != L"DeviceReminderMenu")
            continue;
        menu.Text(Text(L"DeviceReminder") + L" · " + (effective == 0 ? Text(L"Off") : to_hstring(effective) + L"%"));
        for (auto const &child : menu.Items())
        {
            if (auto choice = child.try_as<RadioMenuFlyoutItem>())
            {
                const int selected = unbox_value<int>(choice.Tag());
                choice.IsChecked(selected == threshold);
                if (selected < 0)
                    choice.Text(Format(L"FollowGlobal",{{L"value",globalThreshold==0?Text(L"Off"):to_hstring(globalThreshold)+L"%"}}));
            }
            else if (auto text = child.try_as<MenuFlyoutItem>())
            {
                text.Text(note);
                text.Visibility(note.empty() ? Visibility::Collapsed : Visibility::Visible);
            }
        }
    }
}
void MainWindow::SendCommand(WORD id)
{
    PowerDockTrace(L"command",Windows::Data::Json::JsonObject::Parse(L"{\"id\":"+to_hstring(id)+L"}"));
    if (!preview && ParentAlive())
        PostMessageW(host, WM_COMMAND, id, reinterpret_cast<LPARAM>(hwnd));
}
}
