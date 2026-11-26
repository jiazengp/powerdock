#pragma once
#include <winrt/Microsoft.UI.Xaml.Media.Imaging.h>
#include <winrt/Windows.Storage.Streams.h>

struct NotificationIcon {
    winrt::Microsoft::UI::Xaml::FrameworkElement Create(
        std::wstring_view encoded,
        std::wstring_view deviceType,
        winrt::Microsoft::UI::Xaml::Media::SolidColorBrush const &foreground);
    void UpdateContrast(bool highContrast, winrt::Windows::UI::Color foreground) const;

private:
    winrt::Windows::Storage::Streams::IBuffer pixels{nullptr};
    winrt::Microsoft::UI::Xaml::Media::Imaging::WriteableBitmap bitmap{nullptr};
};
