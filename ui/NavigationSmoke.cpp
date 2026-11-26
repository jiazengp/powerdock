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
void MainWindow::AdvanceNavigationSmoke()
{
    auto self=this;
    const auto require=[](bool condition,char const *message) {
        if(!condition) { std::fprintf(stderr,"Navigation regression: %s\n",message);std::fflush(stderr);std::exit(EXIT_FAILURE); }
    };
    const auto settingsVisible=[&] {
        return self->SettingsPage().Visibility()==Visibility::Visible &&
               self->BatteryPage().Visibility()==Visibility::Collapsed &&
               self->Title()==self->Text(L"Settings");
    };
    std::fprintf(stdout,"Navigation step %d\n",self->smokeTicks+1);std::fflush(stdout);
    switch(++self->smokeTicks) {
    case 1: self->OpenFromTray(1);break;
    case 2: require(settingsVisible(),"cold menu did not open settings");self->ShowTrayMenu();break;
    case 3: self->OpenFromTray(1);break;
    case 4: require(settingsVisible(),"warm menu did not preserve settings");self->SettingsPage().SelectedItem(self->NotificationsNav());break;
    case 5: {
        require(self->NotificationsPanel().Visibility()==Visibility::Visible,"notification tab");
        require(unbox_value<hstring>(ToolTipService::GetToolTip(self->StartupSwitch()))==self->Text(L"StartupDescription"),
            "startup explanation missing from tooltip");
        require(Automation::AutomationProperties::GetHelpText(self->MotionSwitch())==self->Text(L"ReduceMotionDescription"),
            "reduce-motion explanation missing from help text");
        require(unbox_value<hstring>(ToolTipService::GetToolTip(self->ThresholdChoice()))==self->Text(L"ReminderDescription"),
            "default reminder explanation missing from tooltip");
        const auto original=self->latestState?self->latestState:Windows::Data::Json::JsonObject{};
        auto state=Windows::Data::Json::JsonObject::Parse(original.Stringify());
        auto notice=Windows::Data::Json::JsonObject::Parse(L"{\"style\":\"overlay\",\"position\":\"bottom_left\",\"duration_secs\":10,\"monitor\":\"primary\",\"transparency_percent\":45}");
        state.Insert(L"notification_preferences",notice);self->updating=true;self->UpdatePreferences(state);
        require(self->OverlayPreferencesPanel().Visibility()==Visibility::Visible,"overlay configuration hidden");
        require(self->OverlayPositionChoice().SelectedIndex()==3 && self->OverlayMonitorChoice().SelectedIndex()==1 && self->OverlayDurationChoice().SelectedIndex()==2,"overlay configuration values");
        require(self->OverlayTransparencySlider().Value()==45 && self->OverlayTransparencyValue().Text()==L"45%","background transparency setting/label");
        require(self->NotificationAfterFullscreenHint().Visibility()==Visibility::Visible,"enabled fullscreen reminder hint missing");
        notice.Insert(L"after_fullscreen",Windows::Data::Json::JsonValue::CreateBooleanValue(false));self->UpdatePreferences(state);
        require(self->NotificationAfterFullscreenHint().Visibility()==Visibility::Collapsed,"disabled fullscreen reminder kept its hint");
        notice.Insert(L"after_fullscreen",Windows::Data::Json::JsonValue::CreateBooleanValue(true));self->UpdatePreferences(state);
        notice.Insert(L"style",Windows::Data::Json::JsonValue::CreateStringValue(L"windows"));self->UpdatePreferences(state);
        require(self->OverlayPreferencesPanel().Visibility()==Visibility::Collapsed,"Windows notifications retained overlay controls");
        auto idle=JsonObject::Parse(original.Stringify());
        idle.Insert(L"updates",JsonObject::Parse(L"{\"state\":\"idle\",\"managed\":false}"));
        idle.Insert(L"update_preferences",JsonObject{});
        self->UpdatePreferences(idle);
        require(self->UpdateCard().Visibility()==Visibility::Collapsed,"empty update card remained visible");
        require(self->SystemProxySwitch().IsOn(),"system proxy did not default to on");
        idle.Insert(L"update_preferences",JsonObject::Parse(L"{\"follow_system_proxy\":false}"));
        idle.Insert(L"updates",JsonObject::Parse(L"{\"state\":\"windows_connection_error\",\"managed\":false}"));
        self->UpdatePreferences(idle);
        require(self->UpdateCard().Visibility()==Visibility::Visible,"update status did not reveal its card");
        require(!self->SystemProxySwitch().IsOn(),"system proxy opt-out did not reach settings");
        require(self->UpdateStatusText().Text()==self->Text(L"UpdateWindowsConnectionError"),"Windows connection error was not localized");
        self->UpdatePreferences(original);self->updating=false;
        self->SettingsPage().SelectedItem(self->AboutNav());break;
    }
    case 6: {
        require(self->AboutPanel().Visibility()==Visibility::Visible,"about tab");
        auto original=JsonObject::Parse(self->latestState?self->latestState.Stringify():L"{\"version\":1}");
        std::string packet = R"({"version":1,"packet_fixture":")";
        packet.resize(32767, 'x');
        packet += "\xE7\x94\xB5";
        packet += R"("})";
        const std::string_view bytes{packet};
        require(!self->ReceiveStatePacket(7, bytes.substr(32768)), "state accepted a final packet without a start");
        require(self->ReceiveStatePacket(5, bytes.substr(0, 32768)), "state rejected the first packet");
        require(!self->latestState || !self->latestState.HasKey(L"packet_fixture"), "partial state leaked into presentation");
        require(self->ReceiveStatePacket(7, bytes.substr(32768)), "state rejected the final packet");
        auto text = self->latestState.GetNamedString(L"packet_fixture");
        require(std::wstring_view{text}.ends_with(L"\u7535"), "UTF-8 character split across packets was lost");
        require(self->ReceiveStatePacket(5, bytes.substr(0, 32768)), "state rejected an interrupted packet");
        require(self->ReceiveStatePacket(1, R"({"version":1})"), "single packet did not replace incomplete state");
        require(!self->ReceiveStatePacket(7, bytes.substr(32768)), "interrupted fragments were retained");
        original.Insert(L"locale",JsonValue::CreateStringValue(self->locale));
        auto state=JsonObject::Parse(L"{\"version\":1,\"devices\":[{\"id\":\"i18n-mouse\",\"name\":\"Mouse %{percent}\",\"kind\":\"mouse\",\"percent\":15,\"available\":true,\"power\":\"Battery\"}]}");
        state.Insert(L"read_budget",JsonObject::Parse(L"{\"budget_deferred\":2}"));
        for(auto const &language:{L"en-US",L"zh-CN",L"en-US"}) {
            const bool english=std::wstring_view{language}==L"en-US";
            state.Insert(L"locale",JsonValue::CreateStringValue(language));
            state.Insert(L"language",JsonValue::CreateStringValue(language));
            self->ReceiveState(to_string(state.Stringify()));
            require(std::wstring_view{self->FleetSummary().Text()}.find(self->Format(L"BudgetDeferredCount",{{L"count",L"2"}}))!=std::wstring_view::npos,"deferred candidate summary missing or not localized");
            require(self->Title()==(english?L"Settings":L"设置"),"localized native settings title");
            require(self->Root().Language()==language,"XAML language qualifier");
            require(self->LanguageChoice().SelectedIndex()==(english?2:1),"language selection drift");
            require(self->Text(L"NotificationOverlay")==(english?L"NVIDIA-style popup":L"NVIDIA 风格弹窗"),"NVIDIA popup style localization");
            require(self->AboutPanel().Visibility()==Visibility::Visible,"language change reset settings tab");
            require(self->Format(L"NotificationBody",{{L"name",L"Mouse %{percent}"},{L"percent",L"15"}})==
                (english?L"Mouse %{percent} is at 15%. Charge it soon.":L"Mouse %{percent} 电量 15%，请充电。"),"interpolation modified literal device name");
            require(Automation::AutomationProperties::GetName(self->SettingsButton())==self->Text(L"Settings"),"language change retained old accessible name");
            require(unbox_value<hstring>(ToolTipService::GetToolTip(self->BackButton()))==self->Text(L"Back"),"language change retained old tooltip");
            require(unbox_value<hstring>(self->ThresholdChoice().Items().GetAt(2).as<ComboBoxItem>().Content())==self->Format(L"BelowThreshold",{{L"value",L"20"}}),"localized threshold choice");
            require(unbox_value<hstring>(self->OverlayDurationChoice().Items().GetAt(2).as<ComboBoxItem>().Content())==self->Format(L"DurationSeconds",{{L"value",L"10"}}),"localized duration choice");
            auto card=self->DeviceRows().Children().GetAt(0).as<FrameworkElement>();
            auto more=card.FindName(L"CardMore").as<Button>();
            require(card.FindName(L"CardName").as<TextBlock>().Text()==L"Mouse %{percent}","language change modified device name");
            require(more.Flyout().as<MenuFlyout>().Items().GetAt(0).as<MenuFlyoutItem>().Text()==self->Text(L"RefreshDevice"),"language change retained old device menu");
        }
        self->ReceiveState(to_string(original.Stringify()));
        self->OpenFromTray(0);break;
    }
    case 7:
        require(self->BatteryPage().Visibility()==Visibility::Visible,"return to overview");
        self->Root().UpdateLayout();
        for(auto const &button:{self->RefreshButton(),self->SortButton(),self->SettingsButton()}) {
            const auto icon=button.Content().as<FontIcon>();
            const auto padding=button.Padding();const auto border=button.BorderThickness();
            const auto contentHeight=button.ActualHeight()-padding.Top-padding.Bottom-border.Top-border.Bottom;
            require(icon.DesiredSize().Height<=contentHeight+.5,"header icon exceeds content height");
            require(icon.ActualHeight()>=icon.DesiredSize().Height-.5,"header icon vertically clipped");
            require(!icon.IsTextScaleFactorEnabled(),"fixed header icon scales beyond its button");
        }
        require(Automation::AutomationProperties::GetName(self->SettingsButton())==self->Text(L"Settings"),"settings button localization");
        Automation::Peers::ButtonAutomationPeer{self->SettingsButton()}
            .GetPattern(Automation::Peers::PatternInterface::Invoke)
            .as<Automation::Provider::IInvokeProvider>().Invoke();
        break;
    case 8:
        require(settingsVisible(),"header settings button did not open settings");
        require(self->BackButton().Visibility()==Visibility::Visible,"settings has no title-bar back button");
        self->Root().UpdateLayout();
        require(self->BackButton().TransformToVisual(self->Root()).TransformPoint({0,0}).X>=16,"title-bar back ignores content alignment inset");
        require(Automation::AutomationProperties::GetName(self->BackButton())==self->Text(L"Back"),"back button localization");
        Automation::Peers::ButtonAutomationPeer{self->BackButton()}
            .GetPattern(Automation::Peers::PatternInterface::Invoke)
            .as<Automation::Provider::IInvokeProvider>().Invoke();
        break;
    case 9:
        require(self->BatteryPage().Visibility()==Visibility::Visible,"title-bar back did not return to overview");
        require(self->BackButton().Visibility()==Visibility::Collapsed && self->AppTitleBar().Title().empty(),"overview retained settings title/back button");
        self->menuOnly=true;self->AppWindow().Hide();self->ShowTrayMenu();break;
    case 10:
        std::fputs("WinUI navigation passed\n",stdout);std::fflush(stdout);
        self->CloseTrayMenu();break;
    case 13: require(false,"dismissed menu left frontend running");break;
    }
    return;
}
}
