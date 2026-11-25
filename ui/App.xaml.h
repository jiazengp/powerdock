#pragma once
#include "App.xaml.g.h"
#include "NotificationWindow.h"
namespace winrt::PowerDockUI::implementation
{
struct App : AppT<App>
{
    App();
    void OnLaunched(Microsoft::UI::Xaml::LaunchActivatedEventArgs const &);

  private:
    Microsoft::UI::Xaml::Window window{nullptr};
    std::shared_ptr<NotificationWindow> notification;
};
} // namespace winrt::PowerDockUI::implementation
