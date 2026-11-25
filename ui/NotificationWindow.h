#pragma once
#include <memory>
#include <chrono>
#include <vector>
#include <string>
#include <winrt/Microsoft.UI.Composition.h>
#include <winrt/Windows.UI.Composition.h>
#include <winrt/Windows.UI.ViewManagement.h>

struct NotificationWindow : std::enable_shared_from_this<NotificationWindow> {
    winrt::Microsoft::UI::Xaml::Window Show(std::vector<std::wstring> const &arguments);
private:
    enum class Phase { Preparing, Entering, Visible, Exiting, Closed };
    winrt::Microsoft::UI::Xaml::Window window{nullptr};
    winrt::Microsoft::UI::Xaml::Controls::Grid viewport{nullptr};
    winrt::Microsoft::UI::Xaml::Controls::Border backplate{nullptr}, face{nullptr};
    winrt::Microsoft::UI::Xaml::DispatcherTimer timer{nullptr}, watchdog{nullptr};
    winrt::Microsoft::UI::Composition::CompositionScopedBatch motion{nullptr};
    winrt::Microsoft::UI::Composition::Visual backVisual{nullptr}, faceVisual{nullptr};
    winrt::Microsoft::UI::Composition::InsetClip revealClip{nullptr};
    winrt::Microsoft::UI::Composition::ExpressionAnimation revealExpression{nullptr};
    winrt::Windows::UI::Composition::Compositor backdropCompositor{nullptr};
    winrt::Windows::UI::Composition::CompositionColorBrush transparentBackdrop{nullptr};
    winrt::Microsoft::UI::Xaml::Media::CompositionTarget::Rendering_revoker prepared;
    winrt::Windows::UI::ViewManagement::UISettings effectSettings{nullptr};
    winrt::Windows::UI::ViewManagement::UISettings::AdvancedEffectsEnabledChanged_revoker effectsChanged;
    winrt::Windows::UI::ViewManagement::UISettings::ColorValuesChanged_revoker colorsChanged;
    winrt::Microsoft::UI::Xaml::Media::SolidColorBrush backgroundBrush{nullptr}, foregroundBrush{nullptr}, accentBrush{nullptr};
    HWND hwnd{nullptr}, host{nullptr};
    DWORD parent{0};
    bool reduced{false}, smoke{false}, sound{false}, motionProbe{false};
    int preparedFrames{0};
    unsigned long duration{6};
    unsigned long transparencyPercent{20}, effectiveTransparency{0};
    float offscreen{400.f};
    Phase phase{Phase::Preparing};
    std::chrono::steady_clock::time_point phaseStarted{};
    void ConfigureTransparency();
    void UpdateAppearance();
    void SetRevealTracking(bool enabled);
    void Animate(bool exit);
    void FinishMotion(bool exit);
    void Close(bool passed = true);
    static LRESULT CALLBACK Subclass(HWND, UINT, WPARAM, LPARAM, UINT_PTR, DWORD_PTR);
};
