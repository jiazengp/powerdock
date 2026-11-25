#include "pch.h"
#include "NotificationWindow.h"
#include "Diagnostics.h"
#include <winrt/Microsoft.UI.Xaml.Hosting.h>
#include <winrt/Windows.Foundation.Numerics.h>
#include <cstdio>
#include <limits>
#include <cerrno>
#include <cmath>
using namespace winrt;
using namespace Microsoft::UI::Xaml;
using namespace Microsoft::UI::Xaml::Controls;
using namespace Microsoft::UI::Windowing;
using namespace std::chrono;

namespace {
constexpr float popupWidth=320.f, minimumHeight=80.f;
constexpr int enterDurationMs=280, enterStaggerMs=110;
constexpr int exitDurationMs=180, exitStaggerMs=80;
}

// DWM clearing/transparent brush pattern adapted from WinUIEx (MIT).
// See vendor/WINUIEX-MIT.txt and THIRD_PARTY_NOTICES.md.
void NotificationWindow::ConfigureTransparency() {
    const MARGINS margins{};
    check_hresult(DwmExtendFrameIntoClientArea(hwnd, &margins));
    const auto region = CreateRectRgn(-2, -2, -1, -1);
    if (!region) throw hresult_error(HRESULT_FROM_WIN32(GetLastError()));
    DWM_BLURBEHIND blur{DWM_BB_ENABLE | DWM_BB_BLURREGION, TRUE, region, FALSE};
    const auto status = DwmEnableBlurBehindWindow(hwnd, &blur);
    DeleteObject(region);
    check_hresult(status);
}

void NotificationWindow::UpdateAppearance() {
    HIGHCONTRASTW contrast{sizeof(contrast)};
    const bool highContrast=!SystemParametersInfoW(SPI_GETHIGHCONTRAST,sizeof(contrast),&contrast,0)
        || (contrast.dwFlags&HCF_HIGHCONTRASTON)!=0;
    const bool transparent=!highContrast && effectSettings.AdvancedEffectsEnabled();
    effectiveTransparency=transparent?transparencyPercent:0;
    const auto alpha=static_cast<uint8_t>((255*(100-effectiveTransparency)+50)/100);
    const auto systemColor=[](int index) {
        const auto value=GetSysColor(index);
        return Windows::UI::Color{255,GetRValue(value),GetGValue(value),GetBValue(value)};
    };
    backgroundBrush.Color(highContrast?systemColor(COLOR_WINDOW):Windows::UI::Color{alpha,16,16,16});
    accentBrush.Color(highContrast?systemColor(COLOR_HIGHLIGHT):Windows::UI::Color{alpha,15,108,189});
    foregroundBrush.Color(highContrast?systemColor(COLOR_WINDOWTEXT):Windows::UI::Color{255,245,245,245});
}

Window NotificationWindow::Show(std::vector<std::wstring> const &arguments) {
    std::wstring title=L"PowerDock", body, position=L"top-right";
    bool primary=false;
    for (size_t i=1; i<arguments.size(); ++i) {
        auto const &arg=arguments[i];
        if (arg==L"--title" && i+1<arguments.size()) title=arguments[++i];
        else if (arg==L"--body" && i+1<arguments.size()) body=arguments[++i];
        else if (arg==L"--host" && i+1<arguments.size()) host=reinterpret_cast<HWND>(_wcstoui64(arguments[++i].c_str(),nullptr,10));
        else if (arg==L"--parent" && i+1<arguments.size()) parent=wcstoul(arguments[++i].c_str(),nullptr,10);
        else if (arg==L"--reduce-motion") reduced=true;
        else if (arg==L"--smoke-test") smoke=true;
        else if (arg==L"--sound") sound=true;
        else if (arg==L"--position" && i+1<arguments.size()) position=arguments[++i];
        else if (arg==L"--duration" && i+1<arguments.size()) duration=std::clamp(wcstoul(arguments[++i].c_str(),nullptr,10),2ul,30ul);
        else if (arg==L"--transparency") {
            if(i+1>=arguments.size())throw hresult_invalid_argument(L"Missing background transparency");
            const auto &value=arguments[++i];
            wchar_t *end=nullptr;errno=0;
            const auto parsed=wcstoul(value.c_str(),&end,10);
            if(errno==ERANGE || end==value.c_str() || !end || *end!=L'\0' || parsed>100)
                throw hresult_invalid_argument(L"Background transparency must be between 0 and 100");
            transparencyPercent=parsed;
        }
        else if (arg==L"--primary-monitor") primary=true;
        else if (arg==L"--motion-probe") motionProbe=true;
    }
    title.resize(std::min(title.size(),size_t{128}));
    motionProbe=motionProbe && !host && !smoke;
    body.resize(std::min(body.size(),size_t{512}));
    BOOL animations=TRUE;
    SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION,0,&animations,0);
    reduced|=!animations;
    effectSettings=Windows::UI::ViewManagement::UISettings{};
    backgroundBrush=Media::SolidColorBrush{};
    foregroundBrush=Media::SolidColorBrush{};
    accentBrush=Media::SolidColorBrush{};
    UpdateAppearance();
    window=Window{};
    window.Title(L"PowerDock notification");
    auto presenter=OverlappedPresenter::CreateForContextMenu();
    presenter.SetBorderAndTitleBar(false,false);
    presenter.IsAlwaysOnTop(true);
    window.AppWindow().SetPresenter(presenter);
    window.AppWindow().IsShownInSwitchers(motionProbe);
    window.as<IWindowNative>()->get_WindowHandle(&hwnd);
    const auto style=GetWindowLongPtrW(hwnd,GWL_EXSTYLE);
    SetWindowLongPtrW(hwnd,GWL_EXSTYLE,motionProbe?
        (style|WS_EX_NOACTIVATE|WS_EX_APPWINDOW)&~WS_EX_TOOLWINDOW:
        (style|WS_EX_NOACTIVATE|WS_EX_TOOLWINDOW|WS_EX_TRANSPARENT)&~WS_EX_APPWINDOW);
    SetWindowSubclass(hwnd,Subclass,1,reinterpret_cast<DWORD_PTR>(this));
    const auto corner=DWMWCP_DONOTROUND;
    const auto border=DWMWA_COLOR_NONE;
    const BOOL disableSystemMotion=TRUE;
    DwmSetWindowAttribute(hwnd,DWMWA_WINDOW_CORNER_PREFERENCE,&corner,sizeof(corner));
    DwmSetWindowAttribute(hwnd,DWMWA_BORDER_COLOR,&border,sizeof(border));
    DwmSetWindowAttribute(hwnd,DWMWA_TRANSITIONS_FORCEDISABLED,&disableSystemMotion,sizeof(disableSystemMotion));
    window.DispatcherQueue().EnsureSystemDispatcherQueue();
    backdropCompositor=Windows::UI::Composition::Compositor{};
    transparentBackdrop=backdropCompositor.CreateColorBrush({0,0,0,0});
    window.as<Microsoft::UI::Composition::ICompositionSupportsSystemBackdrop>().SystemBackdrop(transparentBackdrop);
    ConfigureTransparency();

    const bool left=position==L"top-left" || position==L"bottom-left";
    const bool bottom=position==L"bottom-left" || position==L"bottom-right";
    viewport=Grid{};
    viewport.IsHitTestVisible(false);
    viewport.RequestedTheme(ElementTheme::Dark);
    viewport.UseLayoutRounding(true);
    backplate=Border{};
    backplate.Background(accentBrush);
    backplate.CornerRadius({0,0,0,0});
    viewport.Children().Append(backplate);
    face=Border{};
    face.CornerRadius({0,0,0,0});
    face.Background(backgroundBrush);
    face.BorderBrush(accentBrush);
    face.BorderThickness(left?Thickness{0,0,4,0}:Thickness{4,0,0,0});
    face.MinHeight(minimumHeight);
    Grid content;
    content.Padding({16,12,16,12});
    content.ColumnSpacing(12);
    ColumnDefinition iconColumn;
    iconColumn.Width({30,GridUnitType::Pixel});
    content.ColumnDefinitions().Append(iconColumn);
    content.ColumnDefinitions().Append(ColumnDefinition{});
    FontIcon icon;
    icon.Glyph(L"\uE83F");icon.FontSize(24);icon.Foreground(foregroundBrush);icon.IsTextScaleFactorEnabled(false);
    content.Children().Append(icon);
    StackPanel text;
    text.Spacing(4);text.VerticalAlignment(VerticalAlignment::Center);Grid::SetColumn(text,1);
    TextBlock heading;
    heading.Text(title);heading.FontSize(15);heading.FontWeight(Windows::UI::Text::FontWeights::SemiBold());
    heading.MaxLines(2);heading.TextTrimming(TextTrimming::CharacterEllipsis);
    heading.Foreground(foregroundBrush);heading.TextWrapping(TextWrapping::Wrap);text.Children().Append(heading);
    TextBlock detail;
    detail.Text(body);detail.FontSize(13);detail.Foreground(foregroundBrush);detail.TextWrapping(TextWrapping::Wrap);
    detail.MaxLines(4);detail.TextTrimming(TextTrimming::CharacterEllipsis);text.Children().Append(detail);
    content.Children().Append(text);
    face.Child(content);
    viewport.Children().Append(face);
    Automation::AutomationProperties::SetName(viewport,hstring{title}+L" "+hstring{body});
    window.Content(viewport);

    MONITORINFO monitor{sizeof(monitor)};
    if (!GetMonitorInfoW(primary?MonitorFromPoint({0,0},MONITOR_DEFAULTTOPRIMARY):MonitorFromWindow(GetForegroundWindow(),MONITOR_DEFAULTTONEAREST),&monitor))
        throw hresult_error(HRESULT_FROM_WIN32(GetLastError()));
    const auto &screen=monitor.rcMonitor;
    SetWindowPos(hwnd,nullptr,screen.left,screen.top,0,0,SWP_NOSIZE|SWP_NOZORDER|SWP_NOACTIVATE);
    const double scale=GetDpiForWindow(hwnd)/96.;
    const auto width=std::min<LONG>(static_cast<int>(popupWidth*scale),screen.right-screen.left);
    const auto logicalWidth=static_cast<float>(width/scale);
    viewport.Measure({logicalWidth,std::numeric_limits<float>::infinity()});
    const auto height=std::clamp(viewport.DesiredSize().Height,minimumHeight,400.f);
    const auto physicalHeight=static_cast<int>(std::ceil(height*scale));
    const auto verticalMargin=static_cast<int>(std::round(16*scale));
    Media::RectangleGeometry clip;
    clip.Rect({0,0,logicalWidth,height});viewport.Clip(clip);
    offscreen=(left?-1.f:1.f)*(logicalWidth+2);
    Hosting::ElementCompositionPreview::SetIsTranslationEnabled(backplate,true);
    Hosting::ElementCompositionPreview::SetIsTranslationEnabled(face,true);
    backVisual=Hosting::ElementCompositionPreview::GetElementVisual(backplate);
    faceVisual=Hosting::ElementCompositionPreview::GetElementVisual(face);
    const auto compositor=backVisual.Compositor();
    revealClip=compositor.CreateInsetClip();
    backVisual.Clip(revealClip);
    // Crop the accent plate at the moving face edge: translucent backgrounds
    // stay disjoint instead of stacking alpha during the wipe.
    revealExpression=compositor.CreateExpressionAnimation(left?
        L"width - Clamp(back.Translation.X - front.Translation.X, 0, width)":
        L"width - Clamp(front.Translation.X - back.Translation.X, 0, width)");
    revealExpression.SetScalarParameter(L"width",logicalWidth);
    revealExpression.SetReferenceParameter(L"back",backVisual.Properties());
    revealExpression.SetReferenceParameter(L"front",faceVisual.Properties());
    if (motionProbe && !reduced) {
        backVisual.Properties().InsertVector3(L"Translation",{offscreen*.25f,0,0});
        faceVisual.Properties().InsertVector3(L"Translation",{offscreen*.65f,0,0});
        SetRevealTracking(true);
    } else {
        const auto initial=reduced?0.f:offscreen;
        backVisual.Properties().InsertVector3(L"Translation",{initial,0,0});
        faceVisual.Properties().InsertVector3(L"Translation",{initial,0,0});
        SetRevealTracking(false);
    }
    window.AppWindow().MoveAndResize({left?screen.left:screen.right-width,bottom?screen.bottom-physicalHeight-verticalMargin:screen.top+verticalMargin,width,physicalHeight});
    if(smoke) {
        RECT actual{};
        if(!GetWindowRect(hwnd,&actual) ||
           (left?actual.left!=screen.left:actual.right!=screen.right) ||
           (bottom?actual.bottom!=screen.bottom-verticalMargin:actual.top!=screen.top+verticalMargin)) {
            std::fputs("Notification horizontal edge or vertical margin is incorrect\n",stderr);Close(false);
        }
        for(const auto radius:{backplate.CornerRadius(),face.CornerRadius()}) {
            if(radius.TopLeft!=0 || radius.TopRight!=0 || radius.BottomLeft!=0 || radius.BottomRight!=0) {
                std::fputs("Notification corners are not square\n",stderr);Close(false);
            }
        }
        std::fprintf(stdout,"Notification edge alignment: %s-%s, horizontal_gap=0, vertical_gap=%d px (16 DIP), square=1\n",
            bottom?"bottom":"top",left?"left":"right",verticalMargin);
    }

    auto weak=weak_from_this();
    const auto refreshAppearance=[weak](auto const &,auto const &) {
        if(auto self=weak.lock())self->window.DispatcherQueue().TryEnqueue([weak] {
            if(auto current=weak.lock();current && current->phase!=Phase::Closed)current->UpdateAppearance();
        });
    };
    effectsChanged=effectSettings.AdvancedEffectsEnabledChanged(auto_revoke,refreshAppearance);
    colorsChanged=effectSettings.ColorValuesChanged(auto_revoke,refreshAppearance);
    window.Closed([weak](auto const &,auto const &){
        if (auto self=weak.lock()) {
            self->phase=Phase::Closed;self->prepared.revoke();
            self->effectsChanged.revoke();self->colorsChanged.revoke();
            if(self->timer)self->timer.Stop();if(self->watchdog)self->watchdog.Stop();
            self->SetRevealTracking(false);
            self->backVisual.Clip(nullptr);
            self->revealExpression=nullptr;self->revealClip=nullptr;
            self->backVisual=nullptr;self->faceVisual=nullptr;
            self->window.as<Microsoft::UI::Composition::ICompositionSupportsSystemBackdrop>().SystemBackdrop(nullptr);
            self->transparentBackdrop=nullptr;self->backdropCompositor=nullptr;
            RemoveWindowSubclass(self->hwnd,Subclass,1);
        }
        Application::Current().Exit();
    });
    timer=DispatcherTimer{};
    timer.Interval(milliseconds(smoke?500:duration*1000));
    timer.Tick([weak](auto const &,auto const &){if(auto self=weak.lock()){self->timer.Stop();if(self->motionProbe)self->Close();else self->Animate(true);}});
    phaseStarted=steady_clock::now();
    prepared=Media::CompositionTarget::Rendering(auto_revoke,[weak](auto const &,auto const &){
        if(auto self=weak.lock();self && self->motionProbe){self->prepared.revoke();self->phase=Phase::Visible;self->timer.Start();return;}
        if (auto self=weak.lock(); self && ++self->preparedFrames>=2) {
            self->prepared.revoke();self->Animate(false);
        }
    });
    watchdog=DispatcherTimer{};watchdog.Interval(seconds(1));
    watchdog.Tick([weak](auto const &,auto const &){if(auto self=weak.lock()) {
        DWORD pid=0;
        if(self->host && (!IsWindow(self->host) || !GetWindowThreadProcessId(self->host,&pid) || pid!=self->parent)){self->Close();return;}
        if(self->phase!=Phase::Visible && steady_clock::now()-self->phaseStarted>seconds(5)) {
            std::fputs("Notification motion did not complete\n",stderr);self->Close(false);
        }
    }});watchdog.Start();
    window.AppWindow().Show(false);
    SetWindowPos(hwnd,HWND_TOPMOST,0,0,0,0,SWP_NOMOVE|SWP_NOSIZE|SWP_NOACTIVATE);
    return window;
}

void NotificationWindow::SetRevealTracking(bool enabled) {
    const auto property=offscreen<0?L"LeftInset":L"RightInset";
    if(enabled)revealClip.StartAnimation(property,revealExpression);
    else {
        revealClip.StopAnimation(property);
        const auto width=std::abs(offscreen)-2;
        if(offscreen<0)revealClip.LeftInset(width);else revealClip.RightInset(width);
    }
}

void NotificationWindow::Animate(bool exit) {
    if (phase==Phase::Closed || (exit && phase!=Phase::Visible))return;
    phase=exit?Phase::Exiting:Phase::Entering;
    phaseStarted=steady_clock::now();
    if (reduced) {FinishMotion(exit);return;}
    auto compositor=backVisual.Compositor();
    SetRevealTracking(true);
    motion=compositor.CreateScopedBatch(Microsoft::UI::Composition::CompositionBatchTypes::Animation);
    motion.Completed([weak=weak_from_this(),exit](auto const &,auto const &){if(auto self=weak.lock())self->FinishMotion(exit);});
    const auto slide=[&](Microsoft::UI::Composition::Visual const &visual,int durationMs,int delayMs) {
        auto animation=compositor.CreateVector3KeyFrameAnimation();
        animation.Duration(milliseconds(durationMs));animation.DelayTime(milliseconds(delayMs));
        animation.DelayBehavior(Microsoft::UI::Composition::AnimationDelayBehavior::SetInitialValueBeforeDelay);
        // Settle gently on entry; accelerate away on exit instead of braking offscreen.
        auto ease=exit?compositor.CreateCubicBezierEasingFunction({.4f,0},{1,1}):
            compositor.CreateCubicBezierEasingFunction({.2f,0},{.2f,1});
        animation.InsertKeyFrame(0,{exit?0.f:offscreen,0,0});
        animation.InsertKeyFrame(1,{exit?offscreen:0.f,0,0},ease);
        visual.StartAnimation(L"Translation",animation);
    };
    slide(backVisual,exit?exitDurationMs:enterDurationMs,exit?exitStaggerMs:0);
    slide(faceVisual,exit?exitDurationMs:enterDurationMs,exit?0:enterStaggerMs);
    motion.End();
}

void NotificationWindow::FinishMotion(bool exit) {
    if (phase!=(exit?Phase::Exiting:Phase::Entering))return;
    backVisual.StopAnimation(L"Translation");faceVisual.StopAnimation(L"Translation");
    backVisual.Properties().InsertVector3(L"Translation",{exit?offscreen:0.f,0,0});
    faceVisual.Properties().InsertVector3(L"Translation",{exit?offscreen:0.f,0,0});
    SetRevealTracking(false);
    const auto elapsed=duration_cast<milliseconds>(steady_clock::now()-phaseStarted).count();
    Windows::Data::Json::JsonObject trace;trace.Insert(L"elapsed_ms",Windows::Data::Json::JsonValue::CreateNumberValue(static_cast<double>(elapsed)));trace.Insert(L"reduced_motion",Windows::Data::Json::JsonValue::CreateBooleanValue(reduced));PowerDockTrace(exit?L"notification_exit_completed":L"notification_enter_completed",trace);
    if(smoke) {
        std::fprintf(stdout,"Notification %s completed: %lld ms, reduced=%d\n",exit?"exit":"enter",elapsed,reduced);
        std::fflush(stdout);
        const auto expected=exit?exitDurationMs+exitStaggerMs:enterDurationMs+enterStaggerMs;
        if(!reduced && elapsed<expected-20) {std::fputs("Card animation completed prematurely\n",stderr);Close(false);return;}
        if(!exit) {
            const auto content=face.Child().as<Grid>();
            const auto text=content.Children().GetAt(1).as<StackPanel>();
            const auto bounds=text.TransformToVisual(viewport).TransformBounds({0,0,
                static_cast<float>(text.ActualWidth()),static_cast<float>(text.ActualHeight())});
            if(viewport.ActualWidth()>popupWidth+1 || viewport.ActualHeight()<minimumHeight-1 ||
               viewport.ActualHeight()>400 || bounds.X<0 || bounds.Y<0 ||
               bounds.X+bounds.Width>viewport.ActualWidth()+1 ||
               bounds.Y+bounds.Height>viewport.ActualHeight()+1) {
                std::fputs("Notification compact layout clipped its text\n",stderr);Close(false);return;
            }
            std::fprintf(stdout,"Notification layout: %.1f x %.1f, text %.1f x %.1f\n",
                viewport.ActualWidth(),viewport.ActualHeight(),text.ActualWidth(),text.ActualHeight());
        }
    }
    motion=nullptr;
    if(exit){Close();return;}
    phase=Phase::Visible;
    if(sound && !smoke)MessageBeep(MB_ICONEXCLAMATION);
    timer.Start();
}

void NotificationWindow::Close(bool passed) {
    if(phase==Phase::Closed)return;
    if(smoke && passed) {
        const auto style=GetWindowLongPtrW(hwnd,GWL_EXSTYLE);
        if(!(style&WS_EX_NOACTIVATE) || !(style&WS_EX_TOOLWINDOW) || GetForegroundWindow()==hwnd || window.AppWindow().IsShownInSwitchers()) {
            std::fputs("Notification activation policy failed\n",stderr);passed=false;
        }
        const auto alpha=static_cast<uint8_t>((255*(100-effectiveTransparency)+50)/100);
        if(backgroundBrush.Color().A!=alpha || accentBrush.Color().A!=alpha ||
           foregroundBrush.Color().A!=255 || face.Background()!=backgroundBrush || backplate.Background()!=accentBrush ||
           face.Opacity()!=1 || viewport.Opacity()!=1 ||
           (offscreen<0?revealClip.LeftInset():revealClip.RightInset())!=std::abs(offscreen)-2) {
            std::fputs("Notification background transparency policy failed\n",stderr);passed=false;
        }
        std::fprintf(stdout,"Notification transparency: requested=%lu effective=%lu alpha=%u text_alpha=%u\n",
            transparencyPercent,effectiveTransparency,static_cast<unsigned>(alpha),static_cast<unsigned>(foregroundBrush.Color().A));
        if(passed){std::fputs("WinUI notification passed\n",stdout);std::fflush(stdout);}
    }
    if(!passed){std::fflush(stderr);std::exit(EXIT_FAILURE);}
    window.Close();
}

LRESULT CALLBACK NotificationWindow::Subclass(HWND window,UINT message,WPARAM wp,LPARAM lp,UINT_PTR,DWORD_PTR data) {
    if(message==WM_MOUSEACTIVATE)return MA_NOACTIVATE;
    if(message==WM_NCHITTEST)return HTTRANSPARENT;
    if(message==WM_ERASEBKGND) {
        RECT rect{};if(GetClientRect(window,&rect)){FillRect(reinterpret_cast<HDC>(wp),&rect,static_cast<HBRUSH>(GetStockObject(BLACK_BRUSH)));return 1;}
    }
    if(message==WM_DWMCOMPOSITIONCHANGED) {
        try {reinterpret_cast<NotificationWindow*>(data)->ConfigureTransparency();}catch(hresult_error const &){ }
    }
    return DefSubclassProc(window,message,wp,lp);
}
