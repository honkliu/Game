// Settle the CON.txt question: does CreateFileW("CON.txt") create a real file?
// Bob reported "no file created, no error" via .NET; my resv2 reported a
// regular file. Both can be true if the difference is which API/path form
// reaches the filesystem. Test the raw Win32 path with and without a directory.
#include <windows.h>
#include <stdio.h>

static void probe(const wchar_t* label, const wchar_t* dir, const wchar_t* name) {
    wchar_t path[MAX_PATH];
    swprintf(path, MAX_PATH, L"%s\\%s", dir, name);
    DWORD before = GetFileAttributesW(path);
    HANDLE h = CreateFileW(path, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS,
                           FILE_ATTRIBUTE_NORMAL, NULL);
    DWORD lastErr = GetLastError();
    DWORD written = 0;
    BOOL okWrite = FALSE;
    if (h != INVALID_HANDLE_VALUE) {
        okWrite = WriteFile(h, "abc", 3, &written, NULL);
        CloseHandle(h);
    }
    DWORD after = GetFileAttributesW(path);
    BOOL existedBefore = before != INVALID_FILE_ATTRIBUTES;
    BOOL existsAfter = after != INVALID_FILE_ATTRIBUTES;
    wprintf(L"%-10s %-10s open=%ls err=%lu wrote=%lu(%ls) existed_before=%ls exists_after=%ls\n",
            label, name,
            h == INVALID_HANDLE_VALUE ? L"FAIL" : L"OK",
            lastErr, written, okWrite ? L"ok" : L"fail",
            existedBefore ? L"Y" : L"N", existsAfter ? L"Y" : L"N");
    if (existsAfter && !existedBefore) DeleteFileW(path);
}

int main(void) {
    wchar_t dir[MAX_PATH];
    GetTempPathW(MAX_PATH, dir);
    wcscat_s(dir, MAX_PATH, L"consettle");
    CreateDirectoryW(dir, NULL);
    wprintf(L"dir=%ls\n\n", dir);
    const wchar_t* names[] = {
        L"CON", L"CON.txt", L"NUL", L"NUL.txt", L"AUX", L"AUX.txt",
        L"PRN", L"PRN.md", L"COM1", L"COM1.txt", L"COM10.log",
        L"CONIN$", L"CONIN$.log", L"LPT1.log", L"plain.txt",
    };
    wprintf(L"--- with directory prefix ---\n");
    for (unsigned i = 0; i < sizeof(names) / sizeof(names[0]); i++) probe(L"dir", dir, names[i]);

    wprintf(L"\n--- relative (current dir), no prefix ---\n");
    SetCurrentDirectoryW(dir);
    for (unsigned i = 0; i < sizeof(names) / sizeof(names[0]); i++) probe(L"rel", L".", names[i]);
    return 0;
}