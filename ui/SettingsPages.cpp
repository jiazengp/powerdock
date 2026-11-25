#include "pch.h"
#include "MainWindow.xaml.h"
#include <winrt/Microsoft.UI.Xaml.Hosting.h>
#include <winrt/Microsoft.UI.Composition.h>
#include <winrt/Windows.Foundation.Numerics.h>
#include "Diagnostics.h"
#include <cmath>
#include <cwchar>
using namespace winrt;
using namespace Microsoft::UI::Xaml;
using namespace Microsoft::UI::Xaml::Controls;
using namespace Windows::Data::Json;
namespace winrt::PowerDockUI::implementation {
namespace {
constexpr int overlayDurations[]{3, 6, 10, 15, 30};
hstring LocalLogTime(hstring const &timestamp) {
    const std::wstring_view value{timestamp.c_str(),timestamp.size()};
    if(value.size()<19||value[4]!=L'-'||value[7]!=L'-'||value[10]!=L'T'||value[13]!=L':'||value[16]!=L':')return timestamp;
    const auto number=[&](size_t offset,size_t length,int &result){
        int parsed=0;
        for(size_t index=0;index<length;++index){
            const wchar_t digit=value[offset+index];
            if(digit<L'0'||digit>L'9')return false;
            parsed=parsed*10+(digit-L'0');
        }
        result=parsed;
        return true;
    };
    int year{},month{},day{},hour{},minute{},second{};
    if(!number(0,4,year)||!number(5,2,month)||!number(8,2,day)||!number(11,2,hour)||!number(14,2,minute)||!number(17,2,second))return timestamp;
    SYSTEMTIME utc{static_cast<WORD>(year),static_cast<WORD>(month),0,static_cast<WORD>(day),static_cast<WORD>(hour),static_cast<WORD>(minute),static_cast<WORD>(second),0};
    SYSTEMTIME local{};
    if(!SystemTimeToTzSpecificLocalTime(nullptr,&utc,&local))return timestamp;
    wchar_t text[6]{};
    std::swprintf(text,6,L"%02u:%02u",local.wHour,local.wMinute);
    return hstring{text};
}
std::wstring FormatRecentLogs(JsonArray const &logs) {
    std::wstring text;
    for(auto index=logs.Size();index>0;--index){
        auto const entry=logs.GetAt(index-1);
        if(entry.ValueType()!=JsonValueType::Object)continue;
        auto line=entry.GetObject();
        const auto level=line.GetNamedString(L"level",L"");
        const auto event=line.GetNamedString(L"event",L"");
        if(event.empty())continue;
        if(!text.empty())text.append(L"\n");
        text.append(LocalLogTime(line.GetNamedString(L"timestamp",L"")));
        if(!level.empty()){text.append(L"  ");text.append(level);}
        text.append(L"  ");
        text.append(event);
        const auto details=line.GetNamedString(L"details",L"");
        if(!details.empty()){text.append(L" · ");text.append(details);}
    }
    return text;
}
}
hstring MainWindow::Text(hstring const &key) const {
    try {return resources.GetValue(key,resourceContext).ValueAsString();}
    catch(hresult_error const &) {return key;}
}
hstring MainWindow::Format(hstring const &key,std::initializer_list<std::pair<std::wstring,hstring>> values) const {
    const auto localized=Text(key);
    const std::wstring_view source{localized};
    std::wstring result;
    result.reserve(source.size());
    size_t offset=0;
    // Scan only the MRT template; inserted names or diagnostics are literal text.
    while(offset<source.size()) {
        const auto start=source.find(L"%{",offset);
        if(start==std::wstring_view::npos)break;
        const auto end=source.find(L'}',start+2);
        if(end==std::wstring_view::npos)break;
        result.append(source.substr(offset,start-offset));
        const auto name=source.substr(start+2,end-start-2);
        const auto value=std::find_if(values.begin(),values.end(),[&](auto const &entry){return entry.first==name;});
        if(value!=values.end())result.append(value->second);
        else result.append(source.substr(start,end-start+1));
        offset=end+1;
    }
    result.append(source.substr(offset));
    return hstring{result};
}
void MainWindow::ApplyLabels() {
    using Automation::AutomationProperties;
    std::function<void(IInspectable const &)> visit;
    visit=[&](IInspectable const &value){
        auto element=value.try_as<FrameworkElement>();if(!element)return;
        auto tag=element.Tag().try_as<Windows::Foundation::IPropertyValue>();
        if(tag && tag.Type()==Windows::Foundation::PropertyType::String){
            std::wstring key{tag.GetString()};
            if(key.starts_with(L"loc:")){
                auto label=Text(hstring{key.substr(4)});
                if(auto text=element.try_as<TextBlock>())text.Text(label);
                else if(auto menu=element.try_as<MenuFlyoutItem>())menu.Text(label);
                else if(auto toggle=element.try_as<ToggleSwitch>()){TextBlock header;header.Text(label);header.TextWrapping(TextWrapping::Wrap);toggle.Header(header);toggle.OnContent(box_value(hstring{}));toggle.OffContent(box_value(hstring{}));}
                else if(auto combo=element.try_as<ComboBox>()){TextBlock header;header.Text(label);header.TextWrapping(TextWrapping::Wrap);combo.Header(header);}
                else if(auto number=element.try_as<NumberBox>())number.Header(box_value(label));
                else if(auto expander=element.try_as<Expander>())expander.Header(box_value(label));
                else if(auto content=element.try_as<ContentControl>()){
                    if(!content.Content().try_as<FontIcon>())content.Content(box_value(label));
                    if(element.try_as<Button>())ToolTipService::SetToolTip(element,box_value(label));
                }
                AutomationProperties::SetName(element,label);
            }
        }
        if(auto panel=element.try_as<Panel>()){for(auto const &child:panel.Children())visit(child);}
        else if(auto border=element.try_as<Border>()){if(border.Child())visit(border.Child());}
        else if(auto nav=element.try_as<NavigationView>()){for(auto const &item:nav.MenuItems())visit(item);for(auto const &item:nav.FooterMenuItems())visit(item);visit(nav.Content());}
        else if(auto items=element.try_as<ItemsControl>()){for(auto const &item:items.Items())visit(item);}
        else if(auto content=element.try_as<ContentControl>())visit(content.Content());
        if(auto button=element.try_as<Button>();button && button.Flyout()){
            if(auto flyout=button.Flyout().try_as<MenuFlyout>()){for(auto const &item:flyout.Items())visit(item);}
        }
    };
    visit(Root());
    Root().Language(locale);RefreshWindowTitle();
    SortBattery().Text(Text(L"SortBattery"));SortType().Text(Text(L"SortType"));SortCharging().Text(Text(L"SortCharging"));
    SortAscending().Text(Text(L"Asc"));SortDescending().Text(Text(L"Desc"));
    for(int index=1;index<4;++index)ThresholdChoice().Items().GetAt(index).as<ComboBoxItem>().Content(box_value(Format(L"BelowThreshold",{{L"value",to_hstring(index*10)}})));
    for (uint32_t index = 0; index < std::size(overlayDurations); ++index)
        OverlayDurationChoice().Items().GetAt(index).as<ComboBoxItem>().Content(
            box_value(Format(L"DurationSeconds", {{L"value", to_hstring(overlayDurations[index])}})));
    // ComboBox caches its selection presenter when an item's Content changes.
    // Refresh it while ReceiveState's updating guard suppresses settings writes.
    for(auto const &combo:{LanguageChoice(),ThemeChoice(),IntervalChoice(),ThresholdChoice(),NotificationStyleChoice(),OverlayPositionChoice(),OverlayMonitorChoice(),OverlayDurationChoice()}){
        const auto selected=combo.SelectedIndex();combo.SelectedIndex(-1);combo.SelectedIndex(selected);
    }
    const auto describe=[this](FrameworkElement const &control,hstring const &key){
        const auto text=Text(key);
        ToolTipService::SetToolTip(control,box_value(text));
        Automation::AutomationProperties::SetHelpText(control,text);
    };
    describe(StartupSwitch(),L"StartupDescription");
    describe(MotionSwitch(),L"ReduceMotionDescription");
    describe(ThresholdChoice(),L"ReminderDescription");
    describe(AutoCheckSwitch(),L"AutoCheckDescription");
    describe(AutoDownloadSwitch(),L"AutoDownloadDescription");
    describe(PreviewSwitch(),L"PreviewDescription");
    describe(NotificationStyleChoice(),L"NotificationFullscreenLimit");
    describe(OverlayTransparencySlider(),L"OverlayTransparencyHint");
    describe(NotificationAfterFullscreenSwitch(),L"NotificationAfterFullscreenHint");
    RefreshTitleBarLayout();
}
void MainWindow::UpdatePreferences(JsonObject const &data){
    auto diagnostics=data.GetNamedObject(L"diagnostics",JsonObject{});
    auto logErrorValue=diagnostics.GetNamedValue(L"initialization_error",JsonValue::CreateNullValue());
    const auto logError=logErrorValue.ValueType()==JsonValueType::String?logErrorValue.GetString():hstring{};
    const int dropped=static_cast<int>(diagnostics.GetNamedNumber(L"dropped_lines",0));
    if(preview){
        DiagnosticsStatusText().Text(Text(L"DiagnosticsPreview"));
        DiagnosticsStatusText().Visibility(Visibility::Visible);
    } else if(!diagnostics.GetNamedBoolean(L"active",false)){
        DiagnosticsStatusText().Text(Format(L"DiagnosticsUnavailable",{{L"error",logError}}));
        DiagnosticsStatusText().Visibility(Visibility::Visible);
    } else if(dropped>0){
        DiagnosticsStatusText().Text(Format(L"DiagnosticsReady",{{L"dropped",to_hstring(dropped)}}));
        DiagnosticsStatusText().Visibility(Visibility::Visible);
    } else {
        DiagnosticsStatusText().Text(hstring{});
        DiagnosticsStatusText().Visibility(Visibility::Collapsed);
    }
    const auto recent=FormatRecentLogs(data.GetNamedArray(L"recent_logs",JsonArray{}));
    RecentLogText().Text(recent);
    RecentLogCard().Visibility(recent.empty()?Visibility::Collapsed:Visibility::Visible);
    CopyLogsButton().IsEnabled(!preview && !recent.empty());
    auto notice=data.GetNamedObject(L"notification_preferences",JsonObject{});
    auto noticeStyle=notice.GetNamedString(L"style",L"windows");
    NotificationStyleChoice().SelectedIndex(noticeStyle==L"overlay"?1:noticeStyle==L"both"?2:0);
    NotificationSoundSwitch().IsOn(notice.GetNamedBoolean(L"sound",true));
    NotificationAfterFullscreenSwitch().IsOn(notice.GetNamedBoolean(L"after_fullscreen",true));
    NotificationAfterFullscreenSwitch().IsEnabled(!preview && noticeStyle!=L"windows");
    NotificationAfterFullscreenHint().Visibility(NotificationAfterFullscreenSwitch().IsOn()?Visibility::Visible:Visibility::Collapsed);
    NotificationSoundSwitch().IsEnabled(!preview);
    OverlayPreferencesPanel().Visibility(noticeStyle==L"windows"?Visibility::Collapsed:Visibility::Visible);
    const auto position=notice.GetNamedString(L"position",L"top_right");
    OverlayPositionChoice().SelectedIndex(position==L"bottom_right"?1:position==L"top_left"?2:position==L"bottom_left"?3:0);
    OverlayMonitorChoice().SelectedIndex(notice.GetNamedString(L"monitor",L"active")==L"primary"?1:0);
    const auto duration = static_cast<int>(notice.GetNamedNumber(L"duration_secs", 6));
    auto found = std::find(std::begin(overlayDurations), std::end(overlayDurations), duration);
    OverlayDurationChoice().SelectedIndex(found == std::end(overlayDurations)
        ? 1 : static_cast<int>(found - std::begin(overlayDurations)));
    OverlayDurationChoice().IsEnabled(!preview);
    OverlayPositionChoice().IsEnabled(!preview);OverlayMonitorChoice().IsEnabled(!preview);
    if(!overlayTransparencyPending)OverlayTransparencySlider().Value(std::clamp(notice.GetNamedNumber(L"transparency_percent",20),0.,100.));
    OverlayTransparencySlider().IsEnabled(!preview);

    auto language=data.GetNamedString(L"language",L"system");LanguageChoice().SelectedIndex(language==L"zh-CN"?1:language==L"en-US"?2:0);
    auto theme=data.GetNamedString(L"theme",L"system");ThemeChoice().SelectedIndex(theme==L"light"?1:theme==L"dark"?2:0);
    if(!preview){
        const auto requested=theme==L"light"?ElementTheme::Light:theme==L"dark"?ElementTheme::Dark:ElementTheme::Default;
        if(Root().RequestedTheme()!=requested)Root().RequestedTheme(requested);
        const auto caption=theme==L"light"?Microsoft::UI::Windowing::TitleBarTheme::Light:theme==L"dark"?Microsoft::UI::Windowing::TitleBarTheme::Dark:Microsoft::UI::Windowing::TitleBarTheme::UseDefaultAppMode;
        if(AppWindow().TitleBar().PreferredTheme()!=caption)AppWindow().TitleBar().PreferredTheme(caption);
        ApplyFrameTheme();
    }
    const bool reduceMotion=data.GetNamedBoolean(L"reduce_motion",false);MotionSwitch().IsOn(reduceMotion);if(reduceMotion)StopPageMotion();
    for(auto const &control:{LanguageChoice(),ThemeChoice(),IntervalChoice(),ThresholdChoice(),NotificationStyleChoice()})control.IsEnabled(!preview);
    for(auto const &control:{StartupSwitch(),MotionSwitch(),AutoCheckSwitch(),AutoDownloadSwitch(),PreviewSwitch()})control.IsEnabled(!preview);
    TestNoticeButton().IsEnabled(!preview);
    auto prefs=data.GetNamedObject(L"update_preferences",JsonObject{});
    AutoCheckSwitch().IsOn(prefs.GetNamedBoolean(L"auto_check",true));AutoDownloadSwitch().IsOn(prefs.GetNamedBoolean(L"auto_download",true));PreviewSwitch().IsOn(prefs.GetNamedBoolean(L"preview",false));
    VersionText().Text(Format(L"Version",{{L"version",data.GetNamedString(L"application_version",L"development")}}));
    HIGHCONTRASTW contrast{sizeof(contrast)};SystemParametersInfoW(SPI_GETHIGHCONTRAST,sizeof(contrast),&contrast,0);
    const bool highContrast=(contrast.dwFlags&HCF_HIGHCONTRASTON)!=0;
    double textScale=1;try{textScale=Windows::UI::ViewManagement::UISettings{}.TextScaleFactor();}catch(hresult_error const &){}
    const bool customTextScale=std::abs(textScale-1.0)>0.005;
    AccessibilityStatus().Text(Format(L"SystemAccessibility",{{L"contrast",Text(highContrast?L"On":L"Off")},{L"scale",to_hstring(static_cast<int>(textScale*100+0.5))}}));
    AccessibilityStatus().Visibility(highContrast||customTextScale?Visibility::Visible:Visibility::Collapsed);
    auto update=data.GetNamedObject(L"updates",JsonObject{});auto state=update.GetNamedString(L"state",preview?L"demo":L"idle");
    hstring key=L"UpdateIdle";
    for(auto const &[value,label]:std::initializer_list<std::pair<hstring,hstring>>{{L"checking",L"UpdateChecking"},{L"downloading",L"UpdateDownloading"},{L"available",L"UpdateAvailable"},{L"ready",L"UpdateReady"},{L"current",L"UpdateCurrent"},{L"waiting_stable",L"UpdateWaitingStable"},{L"legacy",L"UpdateLegacy"},{L"no_feed",L"UpdateNoFeed"},{L"source_unavailable",L"UpdateSourceUnavailable"},{L"error",L"UpdateError"},{L"timeout",L"UpdateTimeout"},{L"demo",L"UpdateDemo"}})if(state==value){key=label;break;}
    auto version=update.GetNamedValue(L"version",JsonValue::CreateNullValue());
    UpdateStatusText().Text(Format(key,{{L"version",version.ValueType()==JsonValueType::String?version.GetString():L""}}));
    const bool busy=state==L"checking"||state==L"downloading";const bool managed=update.GetNamedBoolean(L"managed",false);
    const bool showStatus=state!=L"idle";
    UpdateStatusText().Visibility(showStatus?Visibility::Visible:Visibility::Collapsed);
    const bool showCheck=!preview&&managed&&!busy&&state!=L"ready"&&state!=L"available"&&state!=L"legacy";
    CheckUpdateButton().Visibility(showCheck?Visibility::Visible:Visibility::Collapsed);
    CheckUpdateButton().IsEnabled(showCheck);
    DownloadUpdateButton().Visibility(state==L"available"?Visibility::Visible:Visibility::Collapsed);
    RestartUpdateButton().Visibility(state==L"ready"?Visibility::Visible:Visibility::Collapsed);
    UpdateBuildLink().Visibility(state==L"legacy"?Visibility::Visible:Visibility::Collapsed);
    auto progress=update.GetNamedValue(L"progress",JsonValue::CreateNullValue());const bool known=progress.ValueType()==JsonValueType::Number;
    BOOL systemAnimations=TRUE;SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION,0,&systemAnimations,0);
    UpdateProgress().Visibility(busy && !reduceMotion && systemAnimations?Visibility::Visible:Visibility::Collapsed);
    if(busy && known && (reduceMotion || !systemAnimations))UpdateStatusText().Text(UpdateStatusText().Text()+L" · "+to_hstring(static_cast<int>(progress.GetNumber()))+L"%");
    UpdateProgress().IsIndeterminate(busy && !known && !reduceMotion && systemAnimations);if(known)UpdateProgress().Value(progress.GetNumber());
    Automation::AutomationProperties::SetName(UpdateProgress(),Text(L"Updates"));
    auto notes=update.GetNamedString(L"notes",L"");ReleaseNotesText().Text(notes);NotesExpander().Visibility(notes.empty()?Visibility::Collapsed:Visibility::Visible);
    auto details=update.GetNamedString(L"details",L"");UpdateDetailsText().Text(details);DetailsExpander().Visibility(details.empty()?Visibility::Collapsed:Visibility::Visible);
    const bool showUpdateCard=showStatus||showCheck||state==L"available"||state==L"ready"||state==L"legacy"||!notes.empty()||!details.empty();
    UpdateCard().Visibility(showUpdateCard?Visibility::Visible:Visibility::Collapsed);
}
void MainWindow::PatchPreference(hstring const &name,IJsonValue const &value){
    if(updating||preview||!ParentAlive())return;
    JsonObject trace;trace.Insert(L"preference",JsonValue::CreateStringValue(name));trace.Insert(L"value",value);PowerDockTrace(L"preference_changed",trace);
    JsonObject operation;operation.Insert(L"operation",JsonValue::CreateStringValue(L"general_set"));operation.Insert(name,value);
    auto bytes=to_string(operation.Stringify());COPYDATASTRUCT packet{4,static_cast<DWORD>(bytes.size()),bytes.data()};DWORD_PTR result=0;
    SendMessageTimeoutW(host,WM_COPYDATA,reinterpret_cast<WPARAM>(hwnd),reinterpret_cast<LPARAM>(&packet),SMTO_ABORTIFHUNG,200,&result);
}
void MainWindow::OverlayTransparencyChanged(IInspectable const &,Controls::Primitives::RangeBaseValueChangedEventArgs const &) {
    const auto value=static_cast<int>(std::round(OverlayTransparencySlider().Value()));
    OverlayTransparencyValue().Text(to_hstring(value)+L"%");
    if(updating || preview)return;
    overlayTransparencyValue=value;
    overlayTransparencyPending=true;
    if(!overlayTransparencyTimer) {
        overlayTransparencyTimer=DispatcherTimer{};
        overlayTransparencyTimer.Interval(std::chrono::milliseconds(350));
        overlayTransparencyTimer.Tick([weak=get_weak()](auto const &,auto const &) {
            if(auto self=weak.get()) {
                self->overlayTransparencyTimer.Stop();
                if(self->closed || !self->overlayTransparencyPending)return;
                self->overlayTransparencyPending=false;
                self->PatchPreference(L"overlay_transparency",JsonValue::CreateNumberValue(self->overlayTransparencyValue));
            }
        });
    }
    // Debounce slider movement to avoid settings writes for every pointer event.
    overlayTransparencyTimer.Stop();
    overlayTransparencyTimer.Start();
}
void MainWindow::GeneralChanged(IInspectable const &sender,SelectionChangedEventArgs const &){
    if(updating)return;
    if(sender==NotificationStyleChoice()){constexpr wchar_t const *values[]{L"windows",L"overlay",L"both"};int index=NotificationStyleChoice().SelectedIndex();if(index>=0 && index<3)PatchPreference(L"notification_style",JsonValue::CreateStringValue(values[index]));}
    else if (sender == OverlayDurationChoice()) {
        const auto index = OverlayDurationChoice().SelectedIndex();
        if (index >= 0 && static_cast<size_t>(index) < std::size(overlayDurations))
            PatchPreference(L"overlay_duration", JsonValue::CreateNumberValue(overlayDurations[index]));
    }
    else if(sender==OverlayPositionChoice()){constexpr wchar_t const *values[]{L"top_right",L"bottom_right",L"top_left",L"bottom_left"};int index=OverlayPositionChoice().SelectedIndex();if(index>=0 && index<4)PatchPreference(L"overlay_position",JsonValue::CreateStringValue(values[index]));}
    else if(sender==OverlayMonitorChoice()){PatchPreference(L"overlay_monitor",JsonValue::CreateStringValue(OverlayMonitorChoice().SelectedIndex()==1?L"primary":L"active"));}
    else if(sender==LanguageChoice()){constexpr wchar_t const *values[]{L"system",L"zh-CN",L"en-US"};int index=LanguageChoice().SelectedIndex();if(index>=0 && index<3)PatchPreference(L"language",JsonValue::CreateStringValue(values[index]));}
    else if(sender==ThemeChoice()){constexpr wchar_t const *values[]{L"system",L"light",L"dark"};int index=ThemeChoice().SelectedIndex();if(index>=0 && index<3)PatchPreference(L"theme",JsonValue::CreateStringValue(values[index]));}
}
void MainWindow::PreferenceToggled(IInspectable const &sender,RoutedEventArgs const &){
    if(updating)return;
    if(sender==NotificationSoundSwitch()){PatchPreference(L"notification_sound",JsonValue::CreateBooleanValue(NotificationSoundSwitch().IsOn()));return;}
    if(sender==NotificationAfterFullscreenSwitch()){
        NotificationAfterFullscreenHint().Visibility(NotificationAfterFullscreenSwitch().IsOn()?Visibility::Visible:Visibility::Collapsed);
        PatchPreference(L"notification_after_fullscreen",JsonValue::CreateBooleanValue(NotificationAfterFullscreenSwitch().IsOn()));
        return;
    }

    auto control=sender.as<ToggleSwitch>();hstring key=sender==MotionSwitch()?L"reduce_motion":sender==AutoCheckSwitch()?L"auto_check":sender==AutoDownloadSwitch()?L"auto_download":L"preview_updates";
    PatchPreference(key,JsonValue::CreateBooleanValue(control.IsOn()));
}
void MainWindow::OpenLogsClick(IInspectable const &,RoutedEventArgs const &){SendCommand(106);}
void MainWindow::ExportDiagnosticsClick(IInspectable const &,RoutedEventArgs const &){SendCommand(107);}
void MainWindow::RefreshWindowTitle() {
    const auto label=Text(settingsPage?L"Settings":L"AppName");
    Title(label);
    AppTitleBar().Title(settingsPage?label:L"");
    // The compact localized button uses the public LeftHeader slot.
    AppTitleBar().IsBackButtonVisible(false);
    BackButton().Visibility(settingsPage?Visibility::Visible:Visibility::Collapsed);
    Automation::AutomationProperties::SetName(BackButton(),Text(L"Back"));
    ToolTipService::SetToolTip(BackButton(),box_value(Text(L"Back")));
    Automation::AutomationProperties::SetName(AppTitleBar(),label);
    RefreshTitleBarLayout();
}
void MainWindow::StopPageMotion() {
    if(!animatedPage)return;
    using Microsoft::UI::Composition::ICompositionAnimationBase;
    if(pageFade)animatedPage.StopAnimation(pageFade.as<ICompositionAnimationBase>());
    if(pageSlide)animatedPage.StopAnimation(pageSlide.as<ICompositionAnimationBase>());
    animatedPage.Opacity(1);animatedPage.Translation({0,0,0});
    animatedPage=nullptr;pageFade=nullptr;pageSlide=nullptr;
}
void MainWindow::AnimatePage(FrameworkElement const &page,float direction) {
    StopPageMotion();
    BOOL animations=TRUE;SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION,0,&animations,0);
    if(updating || !hasActivated || !page.XamlRoot() || !animations ||
       (latestState && latestState.GetNamedBoolean(L"reduce_motion",false)))return;
    using Hosting::ElementCompositionPreview;
    auto compositor=ElementCompositionPreview::GetElementVisual(page).Compositor();
    auto easing=compositor.CreateCubicBezierEasingFunction({0.2f,0},{0,1});
    auto fade=compositor.CreateScalarKeyFrameAnimation();
    fade.InsertKeyFrame(0,0);fade.InsertKeyFrame(1,1,easing);fade.Duration(std::chrono::milliseconds(150));
    auto slide=compositor.CreateVector3KeyFrameAnimation();
    slide.InsertKeyFrame(0,{direction*12,direction==0?8.0f:0.0f,0});
    slide.InsertKeyFrame(1,{0,0,0},easing);slide.Duration(std::chrono::milliseconds(150));
    // XAML owns these rendering properties. Use its supported animation facade
    // instead of looking for a Translation property on Composition Visual.
    fade.Target(L"Opacity");slide.Target(L"Translation");
    page.Opacity(1);page.Translation({0,0,0});
    animatedPage=page;pageFade=fade;pageSlide=slide;
    page.StartAnimation(fade);page.StartAnimation(slide);
}
void MainWindow::SettingsNavigationChanged(NavigationView const &sender,NavigationViewSelectionChangedEventArgs const &) {
    auto item=sender.SelectedItem();
    const int next=item==GeneralNav()?0:item==NotificationsNav()?1:2;
    const int previous=selectedSettingsTab;selectedSettingsTab=next;
    if(previous!=next)PowerDockTrace(L"settings_tab",JsonObject::Parse(L"{\"tab\":"+to_hstring(next)+L"}"));
    GeneralPanel().Visibility(next==0?Visibility::Visible:Visibility::Collapsed);
    NotificationsPanel().Visibility(next==1?Visibility::Visible:Visibility::Collapsed);
    AboutPanel().Visibility(next==2?Visibility::Visible:Visibility::Collapsed);
    if(previous!=next) {
        SettingsScroll().ChangeView(nullptr,box_value(0.0).as<Windows::Foundation::IReference<double>>(),nullptr,true);
        AnimatePage(next==0?GeneralPanel().as<FrameworkElement>():next==1?NotificationsPanel().as<FrameworkElement>():AboutPanel().as<FrameworkElement>(),next>previous?1.0f:-1.0f);
    }
}

void MainWindow::CheckUpdatesClick(IInspectable const &,RoutedEventArgs const &){SendCommand(700);}
void MainWindow::DownloadUpdatesClick(IInspectable const &,RoutedEventArgs const &){SendCommand(701);}
void MainWindow::ApplyUpdatesClick(IInspectable const &,RoutedEventArgs const &){SendCommand(702);}
fire_and_forget MainWindow::LicensesClick(IInspectable const &,RoutedEventArgs const &){
    auto lifetime=get_strong();
    wchar_t path[32768]{};GetModuleFileNameW(nullptr,path,32768);auto base=std::filesystem::path{path}.parent_path().parent_path();
    std::string notices;
    for(auto const &name:{L"LICENSE",L"THIRD_PARTY_NOTICES.md"}){
        std::ifstream file(base/name,std::ios::binary);if(file){std::string text(65536,'\0');file.read(text.data(),text.size());text.resize(static_cast<size_t>(file.gcount()));notices+=to_string(hstring{name})+"\n\n"+text+"\n\n";}
    }
    TextBlock text;text.Text(to_hstring(notices.empty()?"License files are unavailable in this development layout.":notices));text.TextWrapping(TextWrapping::Wrap);text.IsTextSelectionEnabled(true);
    ScrollViewer scroll;scroll.Content(text);scroll.MaxHeight(400);scroll.MaxWidth(560);
    ContentDialog dialog;dialog.XamlRoot(Root().XamlRoot());dialog.RequestedTheme(Root().ActualTheme());dialog.Title(box_value(Text(L"LicenseTitle")));dialog.CloseButtonText(Text(L"Dismiss"));dialog.Content(scroll);
    try{co_await dialog.ShowAsync();}catch(hresult_error const &){}
}
void MainWindow::CopyDiagnosticsClick(IInspectable const &,RoutedEventArgs const &){
    if(!latestState)return;
    // Use the already-redacted recent records; never gather device paths or history.
    const auto text=FormatRecentLogs(latestState.GetNamedArray(L"recent_logs",JsonArray{}));
    if(text.empty())return;
    using namespace Windows::ApplicationModel::DataTransfer;
    DataPackage package;package.SetText(text);
    try{Clipboard::SetContent(package);}catch(hresult_error const &){return;}
    DiagnosticsFeedbackText().Text(Text(L"Copied"));
    DiagnosticsFeedbackText().Visibility(Visibility::Visible);
    if(!diagnosticsFeedbackTimer){
        diagnosticsFeedbackTimer=DispatcherTimer{};
        diagnosticsFeedbackTimer.Interval(std::chrono::seconds(2));
        diagnosticsFeedbackTimer.Tick([weak=get_weak()](auto const &,auto const &){
            if(auto self=weak.get()){
                self->DiagnosticsFeedbackText().Visibility(Visibility::Collapsed);
                self->diagnosticsFeedbackTimer.Stop();
            }
        });
    }
    diagnosticsFeedbackTimer.Stop();
    diagnosticsFeedbackTimer.Start();
}
}
