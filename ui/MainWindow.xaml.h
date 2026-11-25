#pragma once
#include "MainWindow.g.h"
#include <map>
#include <chrono>
namespace winrt::PowerDockUI::implementation
{
struct MainWindow : MainWindowT<MainWindow>
{
    MainWindow();
    ~MainWindow();
    void RefreshClick(Windows::Foundation::IInspectable const &,
                      Microsoft::UI::Xaml::RoutedEventArgs const &);
    void SettingsClick(Windows::Foundation::IInspectable const &,
                       Microsoft::UI::Xaml::RoutedEventArgs const &);
    void BackClick(Windows::Foundation::IInspectable const &,
                   Microsoft::UI::Xaml::RoutedEventArgs const &);
    void TestNotificationClick(Windows::Foundation::IInspectable const &,
                               Microsoft::UI::Xaml::RoutedEventArgs const &);
    void ThresholdChanged(Windows::Foundation::IInspectable const &,
                          Microsoft::UI::Xaml::Controls::SelectionChangedEventArgs const &);
    void IntervalChanged(Windows::Foundation::IInspectable const &,
                         Microsoft::UI::Xaml::Controls::SelectionChangedEventArgs const &);
    void StartupChanged(Windows::Foundation::IInspectable const &,
                        Microsoft::UI::Xaml::RoutedEventArgs const &);
    void SortClick(Windows::Foundation::IInspectable const &,
                   Microsoft::UI::Xaml::RoutedEventArgs const &);

    void NavigatePage(int page);
    void ActivateInitial();

    void SettingsNavigationChanged(Microsoft::UI::Xaml::Controls::NavigationView const &, Microsoft::UI::Xaml::Controls::NavigationViewSelectionChangedEventArgs const &);
    void GeneralChanged(Windows::Foundation::IInspectable const &, Microsoft::UI::Xaml::Controls::SelectionChangedEventArgs const &);
    void OverlayTransparencyChanged(Windows::Foundation::IInspectable const &, Microsoft::UI::Xaml::Controls::Primitives::RangeBaseValueChangedEventArgs const &);
    void OpenLogsClick(Windows::Foundation::IInspectable const &, Microsoft::UI::Xaml::RoutedEventArgs const &);
    void ExportDiagnosticsClick(Windows::Foundation::IInspectable const &, Microsoft::UI::Xaml::RoutedEventArgs const &);
    void PreferenceToggled(Windows::Foundation::IInspectable const &, Microsoft::UI::Xaml::RoutedEventArgs const &);
    void CheckUpdatesClick(Windows::Foundation::IInspectable const &, Microsoft::UI::Xaml::RoutedEventArgs const &);
    void DownloadUpdatesClick(Windows::Foundation::IInspectable const &, Microsoft::UI::Xaml::RoutedEventArgs const &);
    void ApplyUpdatesClick(Windows::Foundation::IInspectable const &, Microsoft::UI::Xaml::RoutedEventArgs const &);
    winrt::fire_and_forget LicensesClick(Windows::Foundation::IInspectable const &, Microsoft::UI::Xaml::RoutedEventArgs const &);
    void CopyDiagnosticsClick(Windows::Foundation::IInspectable const &, Microsoft::UI::Xaml::RoutedEventArgs const &);

  private:
    HWND hwnd{nullptr}, host{nullptr};
    DWORD parentPid{0};
    RECT anchor{};
    bool hasAnchor{false};
    bool preview{false}, keepOpen{false}, updating{true}, settingsPage{false}, closed{false},
        hasActivated{false};
    bool frameUpdating{false}, frameReady{false};
    Microsoft::UI::Xaml::DispatcherTimer timer{nullptr};
    Microsoft::UI::Xaml::DispatcherTimer overlayTransparencyTimer{nullptr};
    Microsoft::UI::Xaml::DispatcherTimer diagnosticsFeedbackTimer{nullptr};
    bool overlayTransparencyPending{false};
    int overlayTransparencyValue{20};
    void SendCommand(WORD id);
    void ShowPage(bool settings);
    void ShowTrayMenu();
    void CloseTrayMenu();
    void OpenFromTray(int page);
    bool menuOnly{false};
    Microsoft::UI::Xaml::Window trayMenuWindow{nullptr};
    std::string pendingState;
    bool ReceiveStatePacket(ULONG_PTR tag, std::string_view bytes);
    void ReceiveState(std::string_view json);
    bool ParentAlive() const;
    void ApplyFrameTheme() noexcept;
    void PlaceWindow();
    void LayoutDevices();
    void RefreshChargingPresentation();
    void AdvanceChargingSmoke();
    void AdvanceNavigationSmoke();
    struct ChargingState {
        bool observed{false}, charging{false};
        std::chrono::steady_clock::time_point hintUntil{};
    };
    std::map<std::wstring, ChargingState> chargingStates;
    bool chargingFeedbackActive{false};
    bool frameProbe{false}, frameMarker{true};
    void RefreshTitleBarLayout();
    void RefreshDevice(hstring const &id);
    void SetDeviceReminder(hstring const &id, int threshold);
    Microsoft::UI::Xaml::Controls::MenuFlyoutSubItem CreateReminderMenu(hstring const &id);
    void UpdateReminderMenu(Microsoft::UI::Xaml::FrameworkElement const &card,
                            Windows::Data::Json::JsonObject const &device, int globalThreshold);
    Microsoft::Windows::ApplicationModel::Resources::ResourceManager resourceManager{nullptr};
    Microsoft::Windows::ApplicationModel::Resources::ResourceMap resources{nullptr};
    Microsoft::Windows::ApplicationModel::Resources::ResourceContext resourceContext{nullptr};
    hstring locale;
    Windows::Data::Json::JsonObject latestState{nullptr};
    hstring Text(hstring const &key) const;
    hstring Format(hstring const &key, std::initializer_list<std::pair<std::wstring, hstring>> values) const;
    void ApplyLabels();
    void RefreshWindowTitle();
    void AnimatePage(Microsoft::UI::Xaml::FrameworkElement const &, float direction);
    void StopPageMotion();
    Microsoft::UI::Xaml::FrameworkElement animatedPage{nullptr};
    Windows::Foundation::IInspectable pageFade{nullptr}, pageSlide{nullptr};
    int selectedSettingsTab{-1};
    void UpdatePreferences(Windows::Data::Json::JsonObject const &);
    void PatchPreference(hstring const &, Windows::Data::Json::IJsonValue const &);
    bool placed{false};
    bool smokeTest{false};
    bool smokeNavigation{false};
    bool smokeCharging{false};
    int smokeTicks{0};
    static LRESULT CALLBACK Subclass(HWND, UINT, WPARAM, LPARAM, UINT_PTR, DWORD_PTR);
};
} // namespace winrt::PowerDockUI::implementation
namespace winrt::PowerDockUI::factory_implementation
{
struct MainWindow : MainWindowT<MainWindow, implementation::MainWindow>
{
};
} // namespace winrt::PowerDockUI::factory_implementation
