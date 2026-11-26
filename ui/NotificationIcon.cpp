#include "pch.h"
#include "NotificationIcon.h"
#include <winrt/Windows.Security.Cryptography.h>

using namespace winrt;
using namespace Microsoft::UI::Xaml;
using namespace Microsoft::UI::Xaml::Controls;

namespace {
// The tray renderer in src/icon/notification.rs supplies premultiplied BGRA.
constexpr int pixelSide = 64;
constexpr uint32_t pixelBytes = pixelSide * pixelSide * 4;
constexpr size_t encodedLength = ((pixelBytes + 2) / 3) * 4;
constexpr double iconSize = 28;
}

FrameworkElement NotificationIcon::Create(
    std::wstring_view encoded, std::wstring_view deviceType, Media::SolidColorBrush const &foreground)
{
    if (encoded.size() == encodedLength) {
        try {
            auto decoded = Windows::Security::Cryptography::CryptographicBuffer::DecodeFromBase64String(
                hstring{encoded});
            if (decoded.Length() == pixelBytes) {
                auto source = Media::Imaging::WriteableBitmap(pixelSide, pixelSide);
                Image image;
                image.Width(iconSize);
                image.Height(iconSize);
                image.HorizontalAlignment(HorizontalAlignment::Center);
                image.VerticalAlignment(VerticalAlignment::Center);
                image.Source(source);
                if (deviceType.empty()) {
                    Automation::AutomationProperties::SetAccessibilityView(image, Automation::Peers::AccessibilityView::Raw);
                } else {
                    Automation::AutomationProperties::SetName(image, hstring{deviceType});
                    Automation::AutomationProperties::SetAccessibilityView(image, Automation::Peers::AccessibilityView::Content);
                }
                bitmap = source;
                pixels = decoded;
                return image;
            }
        } catch (hresult_error const &) {
            // A missing or unusable bitmap must not suppress the notification.
        }
    }

    FontIcon fallback;
    fallback.Glyph(L"\uE83F");
    fallback.FontSize(24);
    fallback.Foreground(foreground);
    fallback.IsTextScaleFactorEnabled(false);
    Automation::AutomationProperties::SetAccessibilityView(fallback, Automation::Peers::AccessibilityView::Raw);
    return fallback;
}

void NotificationIcon::UpdateContrast(bool highContrast, Windows::UI::Color foreground) const
{
    if (!bitmap)
        return;

    auto target = bitmap.PixelBuffer();
    auto *destination = target.data();
    const auto *source = pixels.data();
    if (highContrast) {
        for (size_t i = 0; i < pixelBytes; i += 4) {
            const auto alpha = source[i + 3];
            destination[i] = static_cast<uint8_t>((foreground.B * alpha + 127) / 255);
            destination[i + 1] = static_cast<uint8_t>((foreground.G * alpha + 127) / 255);
            destination[i + 2] = static_cast<uint8_t>((foreground.R * alpha + 127) / 255);
            destination[i + 3] = alpha;
        }
    } else {
        std::copy_n(source, pixelBytes, destination);
    }
    bitmap.Invalidate();
}
