#include "pch.h"
#include "MainWindow.xaml.h"
#include <set>
#include <cstdio>
#include <limits>
#include <cmath>

using namespace winrt;
using namespace Microsoft::UI::Xaml;
using namespace Microsoft::UI::Xaml::Controls;
using namespace Windows::Data::Json;

namespace winrt::PowerDockUI::implementation
{
void MainWindow::RefreshChargingPresentation()
{
    if(!latestState || closed) return;
    const auto now=std::chrono::steady_clock::now();
    const auto devices=latestState.GetNamedArray(L"devices",JsonArray{});
    BOOL systemAnimations=TRUE;
    SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION,0,&systemAnimations,0);
    const bool animate=systemAnimations && !latestState.GetNamedBoolean(L"reduce_motion",false);
    std::set<std::wstring> live;
    chargingFeedbackActive=false;
    for(uint32_t index=0;index<devices.Size() && index<DeviceRows().Children().Size();++index) {
        const auto device=devices.GetObjectAt(index);
        const auto id=std::wstring{device.GetNamedString(L"id")};
        live.insert(id);
        auto &state=chargingStates[id];
        const auto value=device.GetNamedValue(L"percent",JsonValue::CreateNullValue());
        const auto power=device.GetNamedValue(L"power",JsonValue::CreateNullValue());
        const bool valid=value.ValueType()==JsonValueType::Number &&
            device.GetNamedBoolean(L"available",false) && !device.GetNamedBoolean(L"source_cache",false);
        const auto powerName=power.ValueType()==JsonValueType::String?power.GetString():hstring{};
        const bool charging=valid && powerName==L"Charging";
        // Unknown/stale states do not invent a power transition. A later good
        // response can resume without replaying a false "started charging" hint.
        if(valid && (powerName==L"Charging" || powerName==L"Battery" || powerName==L"Full")) {
            if(charging && (!state.observed || !state.charging))
                state.hintUntil=now+std::chrono::milliseconds(2200);
            state.observed=true;
            state.charging=charging;
        }
        if(!charging) state.hintUntil={};
        const bool announce=charging && now<state.hintUntil;
        chargingFeedbackActive|=announce;
        auto card=DeviceRows().Children().GetAt(index).as<FrameworkElement>();
        auto number=card.FindName(L"CardPercent").as<TextBlock>();
        auto hint=card.FindName(L"CardChargingHint").as<TextBlock>();
        auto glyph=card.FindName(L"CardChargingGlyph").as<Shapes::Path>();
        auto viewport=card.FindName(L"CardChargeViewport").as<Grid>();
        auto numberRow=card.FindName(L"CardPercentRow").as<StackPanel>();
        auto hintRow=card.FindName(L"CardChargingRow").as<StackPanel>();
        auto hintGlyph=card.FindName(L"CardChargingHintGlyph").as<Shapes::Path>();
        if(!viewport.Tag()) {
            hintRow.Translation({0,36,0});
            viewport.Tag(box_value(true));
        }
        for(auto const &text:{numberRow,hintRow}) {
            if(!animate) {
                text.OpacityTransition(nullptr);
                text.TranslationTransition(nullptr);
            } else {
                if(!text.OpacityTransition()) {
                    ScalarTransition transition;
                    transition.Duration(std::chrono::milliseconds(150));
                    text.OpacityTransition(transition);
                }
                if(!text.TranslationTransition()) {
                    Vector3Transition transition;
                    transition.Duration(std::chrono::milliseconds(220));
                    text.TranslationTransition(transition);
                }
            }
        }
        if(charging) hint.Text(Format(L"ChargingPercent",{{L"value",to_hstring(static_cast<int>(value.GetNumber()))}}));
        // Roll each text/bolt pair up to announce charging and back down to the
        // percentage. The clipped viewport prevents either row leaking outside.
        numberRow.Opacity(announce?0:1);
        hintRow.Opacity(announce?1:0);
        glyph.Visibility(charging?Visibility::Visible:Visibility::Collapsed);
        hintGlyph.Visibility(charging?Visibility::Visible:Visibility::Collapsed);
        // Reserve both rows' width so switching back does not clip the outgoing
        // label horizontally. Each bolt remains adjacent within its own row.
        // TextBlock supplies the actual font baseline, including locale/font
        // fallback. Align the filled vector to that baseline at each text size.
        const auto unlimited=std::numeric_limits<float>::infinity();
        number.Measure({unlimited,unlimited});
        hint.Measure({unlimited,unlimited});
        const auto height=std::max(36.f,std::max(number.DesiredSize().Height,hint.DesiredSize().Height));
        viewport.Height(height); // Grow with Windows text scaling; do not clip enlarged fonts.
        numberRow.Translation({0,announce?-height:0.f,0});
        hintRow.Translation({0,announce?0.f:height,0});
        const auto alignBolt=[](TextBlock const &label, Shapes::Path const &bolt) {
            // Baseline includes text scaling; keep the bolt slightly shorter
            // than the visible digits while retaining baseline alignment.
            const auto size=std::max(1.,std::round(label.BaselineOffset()*.70));
            bolt.Height(size);bolt.Width(size*.75);
            bolt.Margin({0,std::max(0.,label.BaselineOffset()-size),0,0});
        };
        alignBolt(number,glyph);alignBolt(hint,hintGlyph);
        numberRow.Measure({unlimited,height});
        hintRow.Measure({unlimited,height});
        const double maxWidth=card.ActualWidth()>0?std::max(1.,card.ActualWidth()-32):260.;
        const auto width=static_cast<float>(std::clamp<double>(std::max(numberRow.DesiredSize().Width,hintRow.DesiredSize().Width),1.,maxWidth));
        viewport.Width(width);
        viewport.Clip().as<Media::RectangleGeometry>().Rect({0,0,width,height});
        // Inherit the same semantic text brush as the percentage; no green override.
        ToolTipService::SetToolTip(glyph,box_value(Text(L"ChargingNow")));
        ToolTipService::SetToolTip(hintGlyph,box_value(Text(L"ChargingNow")));
    }
    std::erase_if(chargingStates,[&](auto const &entry){return !live.contains(entry.first);});
}
void MainWindow::AdvanceChargingSmoke()
{
    BOOL systemAnimations=TRUE;
    SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION,0,&systemAnimations,0);
    const auto require=[](bool condition,char const *message) {
        if(!condition){std::fprintf(stderr,"Charging regression: %s\n",message);std::fflush(stderr);std::exit(EXIT_FAILURE);}
    };
    const auto snapshot=[&](hstring const &power,int percent,bool cached=false,bool reduced=false,hstring const &language=L"zh-CN") {
        JsonObject device;device.Insert(L"id",JsonValue::CreateStringValue(L"charging-fixture"));
        device.Insert(L"name",JsonValue::CreateStringValue(L"Charging fixture (simulated)"));
        device.Insert(L"kind",JsonValue::CreateStringValue(L"mouse"));
        device.Insert(L"percent",JsonValue::CreateNumberValue(percent));
        device.Insert(L"available",JsonValue::CreateBooleanValue(true));
        device.Insert(L"source_cache",JsonValue::CreateBooleanValue(cached));
        device.Insert(L"power",cached?JsonValue::CreateNullValue():JsonValue::CreateStringValue(power));
        device.Insert(L"battery_tone",JsonValue::CreateStringValue(cached?L"unknown":power==L"Charging"?L"charging":L"normal"));
        JsonArray devices;devices.Append(device);JsonObject state;
        state.Insert(L"version",JsonValue::CreateNumberValue(1));state.Insert(L"devices",devices);
        state.Insert(L"locale",JsonValue::CreateStringValue(language));state.Insert(L"demo",JsonValue::CreateBooleanValue(true));
        state.Insert(L"reduce_motion",JsonValue::CreateBooleanValue(reduced));
        ReceiveState(to_string(state.Stringify()));
    };
    RefreshChargingPresentation();
    const auto card=[&]{return DeviceRows().Children().GetAt(0).as<FrameworkElement>();};
    const auto glyph=[&]{return card().FindName(L"CardChargingGlyph").as<Shapes::Path>();};
    const auto hint=[&]{return card().FindName(L"CardChargingHint").as<TextBlock>();};
    const auto numberRow=[&]{return card().FindName(L"CardPercentRow").as<StackPanel>();};
    const auto hintRow=[&]{return card().FindName(L"CardChargingRow").as<StackPanel>();};
    switch(++smokeTicks) {
    case 1: snapshot(L"Battery",40);require(glyph().Visibility()==Visibility::Collapsed,"battery shows bolt");require(FleetSummary().Visibility()==Visibility::Collapsed,"single-device summary is redundant");require(card().FindName(L"CardState").as<TextBlock>().Visibility()==Visibility::Collapsed,"normal battery state is redundant");break;
    case 2: snapshot(L"Charging",40);require(glyph().Visibility()==Visibility::Visible && hintRow().Opacity()==1,"charging entry");require(hint().Text()==L"正在充电中 40%","Chinese charging label");require(glyph().Data() && glyph().Fill() && glyph().Margin().Top>0,"filled vector size/baseline alignment missing");require(numberRow().Translation().y==-static_cast<float>(card().FindName(L"CardChargeViewport").as<Grid>().Height()) && hintRow().Translation().y==0,"charging did not roll up");require(static_cast<bool>(hintRow().TranslationTransition())==static_cast<bool>(systemAnimations),"charging animation ignores system setting");require(glyph().Parent()==numberRow() && card().FindName(L"CardChargingHintGlyph").as<Shapes::Path>().Parent()==hintRow(),"bolt is not grouped with its label");require(card().FindName(L"CardState").as<TextBlock>().Text().empty() && card().FindName(L"CardState").as<TextBlock>().Visibility()==Visibility::Collapsed,"charging state duplicated the entry hint");require(std::wstring_view{Microsoft::UI::Xaml::Automation::AutomationProperties::GetHelpText(card().FindName(L"CardPercent").as<TextBlock>())}.find(Text(L"ChargingNow"))!=std::wstring_view::npos,"charging semantics missing from help text");break;
    case 3: {
        const auto deadline=chargingStates.at(L"charging-fixture").hintUntil;
        snapshot(L"Charging",41);
        require(chargingStates.at(L"charging-fixture").hintUntil==deadline,"refresh replays entry");
        auto original=card();
        auto state=JsonObject::Parse(latestState.Stringify());
        auto list=state.GetNamedArray(L"devices");
        auto added=JsonObject::Parse(list.GetObjectAt(0).Stringify());
        added.Insert(L"id",JsonValue::CreateStringValue(L"new-device"));
        added.Insert(L"power",JsonValue::CreateStringValue(L"Battery"));
        list.InsertAt(0,added);
        ReceiveState(to_string(state.Stringify()));
        auto created=DeviceRows().Children().GetAt(0);
        require(DeviceRows().Children().GetAt(1)==original,"inserting a device recreated another card");
        require(chargingStates.at(L"charging-fixture").hintUntil==deadline,"insertion replayed charging hint");
        require(FleetSummary().Visibility()==Visibility::Visible && FleetSummary().Text()==Format(L"DeviceCount",{{L"count",L"2"},{L"available",L"2"}}),"multi-device summary missing");
        const auto columns=DeviceRows().ColumnDefinitions().Size();
        require(Grid::GetRow(original)==static_cast<int>(1/columns) && Grid::GetColumn(original)==static_cast<int>(1%columns),"inserted card layout");
        list.RemoveAt(0);list.Append(added);
        ReceiveState(to_string(state.Stringify()));
        require(DeviceRows().Children().GetAt(0)==original && DeviceRows().Children().GetAt(1)==created,"sorting recreated device cards");
        list.RemoveAt(1);
        ReceiveState(to_string(state.Stringify()));
        require(DeviceRows().Children().Size()==1 && card()==original,"removal recreated an unaffected card");
        require(!chargingStates.contains(L"new-device"),"removed device retained animation state");
        require(FleetSummary().Visibility()==Visibility::Collapsed,"single-device summary did not hide");
        break;
    }
    case 7: RefreshChargingPresentation();require(numberRow().Opacity()==1 && hintRow().Opacity()==0 && glyph().Visibility()==Visibility::Visible,"hint did not return to percentage");require(numberRow().Translation().y==0 && hintRow().Translation().y==static_cast<float>(card().FindName(L"CardChargeViewport").as<Grid>().Height()),"percentage did not roll back down");require(card().FindName(L"CardState").as<TextBlock>().Visibility()==Visibility::Collapsed,"charging returned a redundant state label");break;
    case 8: snapshot(L"Charging",41,false,false,L"en-US");require(hint().Text()==L"Charging · 41%" && hintRow().Opacity()==0,"locale/sort rebuild replays entry");break;
    case 9: snapshot(L"Battery",41);require(glyph().Visibility()==Visibility::Collapsed,"unplug did not clear bolt");break;
    case 10: snapshot(L"Charging",41);require(hintRow().Opacity()==1,"replug did not announce");break;
    case 11: snapshot(L"Charging",41,true);require(glyph().Visibility()==Visibility::Collapsed && hintRow().Opacity()==0,"cache invents charging");require(card().FindName(L"CardState").as<TextBlock>().Text()==Text(L"CachedReading"),"cached state missing");break;
    case 12: snapshot(L"Charging",41);require(hintRow().Opacity()==0,"transient unknown replays entry");break;
    case 13: snapshot(L"Battery",41,false,true);break;
    case 14: snapshot(L"Charging",41,false,true);require(!numberRow().OpacityTransition() && !hintRow().OpacityTransition() && !numberRow().TranslationTransition() && !hintRow().TranslationTransition(),"reduced motion still animates");break;
    case 15: {
        auto original=card();
        for(auto const &label:{Text(L"BatteryLow"),Text(L"BatteryAbsent"),hstring{L"--"},hstring{L"≈12%"}}) {
            auto state=JsonObject::Parse(latestState.Stringify());
            auto device=state.GetNamedArray(L"devices").GetObjectAt(0);
            device.Insert(L"percent",JsonValue::CreateNullValue());
            device.Insert(L"available",JsonValue::CreateBooleanValue(false));
            JsonObject battery;battery.Insert(L"display",JsonValue::CreateStringValue(label));
            const bool present=label!=Text(L"BatteryAbsent") && label!=L"--";
            battery.Insert(L"fresh",JsonValue::CreateBooleanValue(true));
            battery.Insert(L"presence",JsonValue::CreateStringValue(present?L"present":L"unknown"));
            device.Insert(L"battery",battery);
            ReceiveState(to_string(state.Stringify()));
            require(card()==original,"precision transition recreated a card");
            require(card().FindName(L"CardPercent").as<TextBlock>().Text()==label,"rich battery label lost");
            require(card().FindName(L"CardProgress").as<ProgressBar>().Visibility()==Visibility::Collapsed,"coarse battery invented a percentage bar");
            require(glyph().Visibility()==Visibility::Collapsed && hintRow().Opacity()==0,"coarse battery replayed percentage charging animation");
            require(FleetSummary().Visibility()==Visibility::Collapsed,"rich battery kept a redundant single-device summary");
        }
        break;
    }
    case 16: std::fputs("WinUI charging passed\n",stdout);std::fflush(stdout);if(keepOpen)smokeCharging=false;else Close();break;
    }
}

}
