// Verify the reserved-name claims in downloads.rs against actual file creation.
#include <windows.h>
#include <stdio.h>

static void check(const wchar_t* dir, const wchar_t* name) {
    wchar_t path[MAX_PATH];
    swprintf(path, MAX_PATH, L"%s\\%s", dir, name);
    HANDLE h = CreateFileW(path, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS,
                           FILE_ATTRIBUTE_NORMAL, NULL);
    if (h == INVALID_HANDLE_VALUE) {
        wprintf(L"%-14s -> CreateFile FAILED err=%lu\n", name, GetLastError());
        return;
    }
    DWORD w = 0;
    WriteFile(h, "x", 1, &w, NULL);
    CloseHandle(h);
    // Did a real file appear?
    WIN32_FILE_ATTRIBUTE_DATA fad;
    BOOL exists = GetFileAttributesExW(path, GetFileExInfoStandard, &fad);
    wprintf(L"%-14s -> opened; file_on_disk=%ls attrs=%s\n", name,
            exists ? L"YES" : L"NO (routed to a device)",
            exists ? (fad.dwFileAttributes & FILE_ATTRIBUTE_DEVICE ? L"DEVICE" : L"regular") : L"-");
    if (exists) DeleteFileW(path);
}

int main(void) {
    wchar_t dir[MAX_PATH];
    GetTempPathW(MAX_PATH, dir);
    wcscat_s(dir, MAX_PATH, L"resv2");
    CreateDirectoryW(dir, NULL);
    wprintf(L"dir=%ls\n", dir);
    const wchar_t* names[] = {
        L"CON.txt", L"COM100.log", L"LPT100.log", L"COM99.log",
        L"COM10.log", L"normal.log", L"CONIN$.log", L"CONOUT$.log",
    };
    for (unsigned i = 0; i < sizeof(names) / sizeof(names[0]); i++) check(dir, names[i]);
    return 0;
}