#pragma once
#include <cstdio>
#include <atomic>
#include <winrt/Windows.Data.Json.h>

inline void PowerDockTrace(wchar_t const *event,winrt::Windows::Data::Json::JsonObject details={}) noexcept {
    try {
        winrt::Windows::Data::Json::JsonObject record;
        FILETIME time{};GetSystemTimePreciseAsFileTime(&time);ULARGE_INTEGER ticks{};ticks.LowPart=time.dwLowDateTime;ticks.HighPart=time.dwHighDateTime;
        static std::atomic_uint64_t sequence{0};
        record.Insert(L"sequence",winrt::Windows::Data::Json::JsonValue::CreateNumberValue(static_cast<double>(++sequence)));
        record.Insert(L"timestamp_unix_ms",winrt::Windows::Data::Json::JsonValue::CreateNumberValue(static_cast<double>(ticks.QuadPart/10000-11644473600000ULL)));
        record.Insert(L"event",winrt::Windows::Data::Json::JsonValue::CreateStringValue(event));
        record.Insert(L"details",details);
        std::fprintf(stdout,"PD_TRACE %s\n",winrt::to_string(record.Stringify()).c_str());
        std::fflush(stdout);
    }catch(...){ }
}

inline LONG WINAPI PowerDockNativeFault(EXCEPTION_POINTERS *exception) noexcept {
    // Keep fatal reporting independent of XAML, JSON allocation and the failed heap.
    char text[256]{};
    const auto address=exception->ExceptionRecord->ExceptionAddress;
    HMODULE module{};
    GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS|GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,reinterpret_cast<LPCWSTR>(address),&module);
    const auto length=std::snprintf(text,sizeof(text),"Native fatal exception code=0x%08lX instruction=%p module_offset=0x%llX\n",exception->ExceptionRecord->ExceptionCode,address,static_cast<unsigned long long>(reinterpret_cast<uintptr_t>(address)-reinterpret_cast<uintptr_t>(module)));
    DWORD written=0;
    WriteFile(GetStdHandle(STD_ERROR_HANDLE),text,static_cast<DWORD>(std::clamp(length,0,static_cast<int>(sizeof(text)-1))),&written,nullptr);
    void *frames[24]{};const auto count=CaptureStackBackTrace(0,24,frames,nullptr);
    for(USHORT index=0;index<count;++index){const auto size=std::snprintf(text,sizeof(text),"Native handler stack frame[%u]=%p\n",index,frames[index]);WriteFile(GetStdHandle(STD_ERROR_HANDLE),text,static_cast<DWORD>(size),&written,nullptr);}
    return EXCEPTION_EXECUTE_HANDLER;
}
