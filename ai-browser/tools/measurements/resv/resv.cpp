// Verify Windows reserved-device-name behaviour claimed in downloads.rs.
// Creates files with those names and checks whether they can be opened normally.
#include <windows.h>
#include <stdio.h>

static void try_name(const wchar_t* dir, const wchar_t* name) {
    wchar_t path[MAX_PATH];
    swprintf(path, MAX_PATH, L"%s\\%s", dir, name);
    HANDLE h = CreateFileW(path, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS,
                           FILE_ATTRIBUTE_NORMAL, NULL);
    if (h == INVALID_HANDLE_VALUE) {
        wprintf(L"%-12s CreateFile FAILED err=%lu\n", name, GetLastError());
    } else {
        DWORD wrote = 0;
        const char* data = "x";
        WriteFile(h, data, 1, &wrote, NULL);
        DWORD err = GetLastError();
        wprintf(L"%-12s opened, wrote=%lu err=%lu\n", name, wrote, err);
        CloseHandle(h);
        DeleteFileW(path);
    }
}

int main(void) {
    wchar_t dir[MAX_PATH];
    GetTempPathW(MAX_PATH, dir);
    wcscat_s(dir, MAX_PATH, L"resvtest");
    CreateDirectoryW(dir, NULL);
    wprintf(L"dir=%ls\n", dir);
    const wchar_t* names[] = {
        L"CON", L"CON.txt", L"PRN", L"AUX", L"NUL", L"NUL.txt",
        L"COM1", L"COM9", L"COM10", L"COM100", L"LPT1", L"LPT9",
        L"CONIN$", L"CONOUT$", L"normal.txt",
        L"CON .txt",        // trailing space before the dot
        L"CON..txt",        // doubled dot
    };
    for (int i = 0; i < (int)(sizeof(names) / sizeof(names[0])); i++) {
        try_name(dir, names[i]);
    }
    return 0;
}