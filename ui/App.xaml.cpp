// Window/application lifecycle adapted from Microsoft's MIT-licensed WindowsAppSDK sample.
#include "pch.h"
#include "App.xaml.h"
#include "MainWindow.xaml.h"
#include "Diagnostics.h"
#include <cstdio>
#include <shobjidl_core.h>
namespace winrt::PowerDockUI::implementation
{
App::App()
{
    InitializeComponent();
    UnhandledException([](auto const &, Microsoft::UI::Xaml::UnhandledExceptionEventArgs const &e) {
        OutputDebugStringW(e.Message().c_str());
        std::fprintf(stderr,"WinUI unhandled exception: %s\n",winrt::to_string(e.Message()).c_str());
        std::fprintf(stderr,"WinUI HRESULT: 0x%08X\n",static_cast<unsigned int>(e.Exception().value));
        std::fflush(stderr);
    });
}
void App::OnLaunched(Microsoft::UI::Xaml::LaunchActivatedEventArgs const &)
{
    int count=0;auto argv=CommandLineToArgvW(GetCommandLineW(),&count);
    std::vector<std::wstring> arguments;for(int i=0;argv && i<count;++i)arguments.emplace_back(argv[i]);if(argv)LocalFree(argv);
    if(std::find(arguments.begin(),arguments.end(),L"--notification")!=arguments.end()) {
        notification=std::make_shared<NotificationWindow>();window=notification->Show(arguments);return;
    }
    window = winrt::make<MainWindow>();
    winrt::get_self<MainWindow>(window.as<winrt::PowerDockUI::MainWindow>())->ActivateInitial();
}
} // namespace winrt::PowerDockUI::implementation

int WINAPI wWinMain(HINSTANCE, HINSTANCE, PWSTR, int)
{
    SetErrorMode(SEM_FAILCRITICALERRORS|SEM_NOGPFAULTERRORBOX);
    SetUnhandledExceptionFilter(PowerDockNativeFault);
    std::set_terminate([]{std::fputs("Native C++ terminate\n",stderr);std::fflush(stderr);std::abort();});
    if(wcsstr(GetCommandLineW(),L"--smoke-crash") && GetEnvironmentVariableW(L"POWERDOCK_DIAGNOSTICS_SMOKE",nullptr,0)>0){RaiseException(0xE0005044,EXCEPTION_NONCONTINUABLE,0,nullptr);return EXIT_FAILURE;}
    try {
    PowerDockTrace(L"ui_process_started");
    SetCurrentProcessExplicitAppUserModelID(L"PowerDock.Native");
    winrt::init_apartment(winrt::apartment_type::single_threaded);
    winrt::Microsoft::UI::Xaml::Application::Start(
        [](auto const &) { winrt::make<winrt::PowerDockUI::implementation::App>(); });
    return 0;
    }catch(winrt::hresult_error const &error){std::fprintf(stderr,"Native startup HRESULT=0x%08X error=%s\n",static_cast<unsigned int>(error.code().value),winrt::to_string(error.message()).c_str());std::fflush(stderr);return EXIT_FAILURE;}
    catch(std::exception const &error){std::fprintf(stderr,"Native startup exception: %s\n",error.what());std::fflush(stderr);return EXIT_FAILURE;}
}
