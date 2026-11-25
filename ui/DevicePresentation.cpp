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
bool MainWindow::ReceiveStatePacket(ULONG_PTR tag, std::string_view bytes)
{
    // Same values as the Rust sender in src/ui/window.rs.
    constexpr size_t packetBytes = 32768;
    constexpr size_t snapshotBytes = 8 * packetBytes;
    constexpr ULONG_PTR single = 1, start = 5, more = 6, end = 7;
    if (bytes.empty() || bytes.size() > packetBytes) {
        pendingState.clear();
        return false;
    }
    if (tag == single) {
        pendingState.clear();
        ReceiveState(bytes);
        return true;
    }
    if (tag == start) {
        pendingState.assign(bytes);
        return true;
    }
    if ((tag != more && tag != end) || pendingState.empty() ||
        pendingState.size() + bytes.size() > snapshotBytes) {
        pendingState.clear();
        return false;
    }
    // A packet can split a multibyte UTF-8 character. Decode only the full snapshot.
    pendingState.append(bytes);
    if (tag == end) {
        std::string complete;
        complete.swap(pendingState);
        ReceiveState(complete);
    }
    return true;
}

void MainWindow::ReceiveState(std::string_view bytes)
{
    if (!bytes.empty() && bytes.back() == '\0')
        bytes.remove_suffix(1);
    auto data = JsonObject::Parse(to_hstring(bytes));
    if (data.GetNamedNumber(L"version", 0) != 1)
        return;
    updating = true;
    struct UpdateScope
    {
        bool &flag;
        ~UpdateScope()
        {
            flag = false;
        }
    } scope{updating};
    latestState=data;
    const auto incomingLocale=data.GetNamedString(L"locale",locale);
    const bool localeChanged=locale!=incomingLocale;
    if(localeChanged){locale=incomingLocale;resourceContext=resourceManager.CreateResourceContext();resourceContext.QualifierValues().Insert(L"Language",locale);ApplyLabels();}
    UpdatePreferences(data);
    RefreshWindowTitle();
    auto glyphFor = [](hstring const &kind) -> hstring {
        if (kind == L"mouse")
            return L"\uE962";
        if (kind == L"keyboard")
            return L"\uE765";
        if (kind == L"headset")
            return L"\uE7F6";
        if (kind == L"phone")
            return L"\uE8EA";
        if (kind == L"gamepad")
            return L"\uE7FC";
        return L"\uE975";
    };
    auto devices = data.GetNamedArray(L"devices", JsonArray{});
    std::map<std::wstring,FrameworkElement> existing;
    for(auto const &child:DeviceRows().Children()) {
        auto card=child.as<FrameworkElement>();
        existing.emplace(std::wstring{unbox_value<hstring>(card.Tag())},card);
    }
    // Locale changes rebuild localized menus; topology/order changes retain
    // existing card objects, focus and charging feedback for unaffected ids.
    for(uint32_t i=DeviceRows().Children().Size();i>0;--i) {
        auto card=DeviceRows().Children().GetAt(i-1).as<FrameworkElement>();
        const auto id=unbox_value<hstring>(card.Tag());
        bool present=false;
        for(auto const &item:devices)if(item.GetObject().GetNamedString(L"id")==id){present=true;break;}
        if(localeChanged || !present)DeviceRows().Children().RemoveAt(i-1);
    }
    if(localeChanged)existing.clear();
    auto cardTemplate =
        Root().Resources().Lookup(box_value(L"DeviceCardTemplate")).as<DataTemplate>();
    uint32_t availableCount = 0;
    uint32_t index = 0;
    for (auto const &item : devices)
    {
        auto device = item.GetObject();
        const auto battery = device.GetNamedObject(L"battery", JsonObject{});
        const bool available = device.GetNamedBoolean(L"available", false);
        if (available || (battery.GetNamedBoolean(L"fresh",false) && battery.GetNamedString(L"presence",L"unknown")==L"present"))
            ++availableCount;
        const auto id=device.GetNamedString(L"id");
        const auto found=existing.find(std::wstring{id});
        const bool newCard=found==existing.end();
        auto card=newCard?cardTemplate.LoadContent().as<FrameworkElement>():found->second;
        uint32_t position=0;
        const bool attached=DeviceRows().Children().IndexOf(card,position);
        if(!attached || position!=index) {
            if(attached)DeviceRows().Children().RemoveAt(position);
            DeviceRows().Children().InsertAt(index,card);
        }
        ++index;
        if (newCard)
        {
            card.Tag(box_value(id));
            auto more = card.FindName(L"CardMore").as<Button>();
            ToolTipService::SetToolTip(more,box_value(Text(L"DeviceActions")));
            Microsoft::UI::Xaml::Automation::AutomationProperties::SetName(
                more, Format(L"DeviceMore",{{L"name",device.GetNamedString(L"name",Text(L"AppName"))}}));
            MenuFlyout menu;
            menu.Placement(Controls::Primitives::FlyoutPlacementMode::BottomEdgeAlignedRight);
            MenuFlyoutItem refresh;
            refresh.Text(Text(L"RefreshDevice"));
            FontIcon refreshIcon;
            refreshIcon.Glyph(L"\uE72C");
            refresh.Icon(refreshIcon);
            refresh.IsEnabled(!preview && device.GetNamedString(L"reader", L"") != L"none");
            refresh.Click([weak = get_weak(), id](auto const &, auto const &) {
                if (auto self = weak.get())
                    self->RefreshDevice(id);
            });
            menu.Items().Append(refresh);
            MenuFlyoutItem copy;
            copy.Text(Text(L"CopyDevice"));
            FontIcon copyIcon;
            copyIcon.Glyph(L"\uE8C8");
            copy.Icon(copyIcon);
            copy.Click([weak = make_weak(card)](auto const &, auto const &) {
                if (auto current = weak.get())
                {
                    using namespace Windows::ApplicationModel::DataTransfer;
                    DataPackage package;
                    auto percent = current.FindName(L"CardPercent").as<TextBlock>();
                    auto text = current.FindName(L"CardName").as<TextBlock>().Text() + L"\r\n" + percent.Text();
                    const auto details = Microsoft::UI::Xaml::Automation::AutomationProperties::GetHelpText(percent);
                    if (!details.empty())
                        text = text + L"\r\n" + details;
                    package.SetText(text);
                    try
                    {
                        Clipboard::SetContent(package);
                    }
                    catch (hresult_error const &error)
                    {
                        OutputDebugStringW(error.message().c_str());
                    }
                }
            });
            menu.Items().Append(copy);
            menu.Items().Append(MenuFlyoutSeparator{});
            menu.Items().Append(CreateReminderMenu(id));
            more.Flyout(menu);
        }
        card.FindName(L"CardMore").as<Button>().Flyout().as<MenuFlyout>().Items().GetAt(0)
            .as<MenuFlyoutItem>().IsEnabled(!preview && device.GetNamedString(L"reader",L"none")!=L"none");
        UpdateReminderMenu(card, device, static_cast<int>(data.GetNamedNumber(L"threshold", 20)));
        card.FindName(L"CardGlyph")
            .as<FontIcon>()
            .Glyph(glyphFor(device.GetNamedString(L"kind", L"other")));
        auto name = card.FindName(L"CardName").as<TextBlock>();
        name.Text(device.GetNamedString(L"name", Text(L"AppName")));
        ToolTipService::SetToolTip(name, box_value(name.Text()));
        auto value = device.Lookup(L"percent");
        const bool known = value.ValueType() == JsonValueType::Number;
        const auto batteryDisplay = battery.GetNamedString(L"display", hstring{L"--"});
        card.FindName(L"CardPercent")
            .as<TextBlock>()
            .Text(known ? to_hstring(static_cast<int>(value.GetNumber())) + L"%" : batteryDisplay);
        Microsoft::UI::Xaml::Automation::AutomationProperties::SetName(
            card.FindName(L"CardPercent").as<TextBlock>(),name.Text()+L" · "+batteryDisplay);
        auto bar = card.FindName(L"CardProgress").as<ProgressBar>();
        bar.Value(known ? std::clamp(value.GetNumber(), 0.0, 100.0) : 0);
        bar.Visibility(known ? Visibility::Visible : Visibility::Collapsed);
        // Native theme resources follow light, dark and contrast themes automatically.
        const auto tone = device.GetNamedString(L"battery_tone", L"unknown");
        if (tone == L"normal")
            bar.ClearValue(FrameworkElement::StyleProperty()); // Keep the existing system accent blue.
        else
            bar.Style(Root().Resources().Lookup(box_value(
                tone == L"charging" ? L"BatteryChargingProgressStyle" :
                tone == L"low" ? L"BatteryLowProgressStyle" : L"BatteryUnknownProgressStyle")).as<Style>());
        bar.Opacity(1.0);
        card.FindName(L"CardUnknownTrack")
            .as<UIElement>()
            .Visibility(known ? Visibility::Collapsed : Visibility::Visible);
        Microsoft::UI::Xaml::Automation::AutomationProperties::SetName(
            bar, Format(L"DeviceBattery",{{L"name",name.Text()}}));
        auto source = device.GetNamedString(L"source", L"");
        const auto cached = device.GetNamedBoolean(L"source_cache",false);
        const auto powerValue = device.GetNamedValue(L"power", JsonValue::CreateNullValue());
        const auto power = powerValue.ValueType() == JsonValueType::String ? powerValue.GetString() : hstring{};
        hstring state;
        bool showState = true;
        if (cached)
            state = Text(L"CachedReading");
        else if (known && available && power == L"Full")
            state = Text(L"Status003");
        else if (known && available)
            showState = false;
        else {
            state = device.GetNamedString(L"state", Text(L"NoReading"));
            if (known && !available)
                state = Text(L"OldReading") + L" · " + state;
        }
        auto stateText = card.FindName(L"CardState").as<TextBlock>();
        stateText.Text(state);
        stateText.Visibility(showState && !state.empty() ? Visibility::Visible : Visibility::Collapsed);
        hstring details = showState ? state
            : known && available && power == L"Charging" ? Text(L"ChargingNow")
            : hstring{};
        if (!source.empty())
            details = details.empty() ? source : details + L" · " + source;
        Microsoft::UI::Xaml::Automation::AutomationProperties::SetHelpText(
            card.FindName(L"CardPercent").as<TextBlock>(), details);
        ToolTipService::SetToolTip(
            card.FindName(L"CardPercent").as<TextBlock>(),
            box_value(details.empty() ? name.Text() : details));
    }
    RefreshChargingPresentation();
    LayoutDevices();
    const auto budget=data.GetNamedObject(L"read_budget",JsonObject{});
    const auto deferred=static_cast<uint32_t>(budget.GetNamedNumber(L"budget_deferred",0));
    const bool busy = data.GetNamedBoolean(L"busy", false);
    hstring summary;
    bool showSummary = true;
    if (busy)
        summary = Text(L"Refreshing");
    else if (devices.Size() == 0 || (devices.Size() == 1 && deferred == 0))
        showSummary = false;
    else {
        summary=Format(devices.Size()==1 && availableCount==1 ? L"DeviceCountOne":L"DeviceCount",{{L"count",to_hstring(devices.Size())},{L"available",to_hstring(availableCount)}});
        if(deferred) summary=summary+L" · "+Format(L"BudgetDeferredCount",{{L"count",to_hstring(deferred)}});
    }
    FleetSummary().Text(summary);
    FleetSummary().Visibility(showSummary ? Visibility::Visible : Visibility::Collapsed);
    EmptyState().Visibility(devices.Size() == 0 ? Visibility::Visible : Visibility::Collapsed);
    const bool demo = data.GetNamedBoolean(L"demo", false);
    const auto detail = demo ? Text(L"PreviewMode") : data.GetNamedString(L"detail", L"");
    DetailText().Text(detail);
    DetailText().Visibility((devices.Size() > 0 || demo) && !detail.empty() &&
                            (demo || data.GetNamedBoolean(L"detail_visible", false))
                                ? Visibility::Visible : Visibility::Collapsed);
    const int threshold = static_cast<int>(data.GetNamedNumber(L"threshold", 20));
    const auto sortBy = data.GetNamedString(L"sort_by", L"battery");
    const bool descending = data.GetNamedString(L"sort_order", L"asc") == L"desc";
    SortBattery().IsChecked(sortBy == L"battery");
    SortType().IsChecked(sortBy == L"type");
    SortCharging().IsChecked(sortBy == L"charging");
    SortAscending().IsChecked(!descending);
    SortDescending().IsChecked(descending);
    SortButton().IsEnabled(!preview);
    const hstring sortLabel = Text(L"Sort")+L": "+Text(sortBy==L"type"?L"SortType":sortBy==L"charging"?L"SortCharging":L"SortBattery")+L" · "+Text(descending?L"Desc":L"Asc");
    ToolTipService::SetToolTip(SortButton(), box_value(sortLabel));
    Microsoft::UI::Xaml::Automation::AutomationProperties::SetName(SortButton(), sortLabel);
    ThresholdChoice().SelectedIndex(threshold == 10   ? 1
                                    : threshold == 20 ? 2
                                    : threshold == 30 ? 3
                                                      : 0);
    const int interval = static_cast<int>(data.GetNamedNumber(L"interval_secs", 60));
    IntervalChoice().SelectedIndex(interval == 30 ? 0 : interval == 120 ? 2 : 1);
    StartupSwitch().IsOn(data.GetNamedBoolean(L"startup", false));
    ToolTipService::SetToolTip(RefreshButton(), box_value(Text(busy?L"Refreshing":L"Refresh")));
    RefreshButton().IsEnabled(!busy && !preview);
    updating = false;
}
}
