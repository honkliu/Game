// CreateFile2 (the modern API) with the flag combination a browser download
// uses, plus a check on whether the bytes reach the file or the console.
#include <windows.h>
#include <stdio.h>

static void probe2(const wchar_t* dir, const wchar_t* name, BOOL expectFile) {
    wchar_t path[MAX_PATH];
    swprintf(path, MAX_PATH, L"%s\\%s", dir, name);
    CREATEFILE2_EXTENDED_PARAMETERS ext = {0};
    ext.dwSize = sizeof(ext);
    ext.dwFileAttributes = FILE_ATTRIBUTE_NORMAL;
    ext.dwFileFlags = FILE_FLAG_OVERLAPPED;      // what async downloads use
    HANDLE h = CreateFile2(path, GENERIC_WRITE, 0, CREATE_ALWAYS, &ext);
    DWORD err = GetLastError();
    DWORD wrote = 0;
    if (h != INVALID_HANDLE_VALUE) {
        OVERLAPPED ov = {0};
        ov.hEvent = CreateEventW(NULL, TRUE, FALSE, NULL);
        WriteFile(h, "PAYLOAD", 7, &wrote, &ov);
        CloseHandle(ov.hEvent);
        CloseHandle(h);
    }
    WIN32_FIND_DATAW fd;
    HANDLE f = FindFirstFileW(path, &fd);
    int listed = f != INVALID_HANDLE_VALUE;
    DWORD size = listed ? fd.nFileSizeLow : 0;
    if (f != INVALID_HANDLE_VALUE) FindClose(f);
    wprintf(L"%-12s CreateFile2 open=%ls err=%lu wrote=%lu listed=%d size=%lu  -> %s\n",
            name, h == INVALID_HANDLE_VALUE ? L"FAIL" : L"ok", err, wrote, listed, size,
            listed ? L"REAL FILE" : L"NOT A FILE");
    if (listed) DeleteFileW(path);
    (void)expectFile;
}

int main(void) {
    wchar_t dir[MAX_PATH];
    GetTempPathW(MAX_PATH, dir);
    wcscat_s(dir, MAX_PATH, L"c2test");
    CreateDirectoryW(dir, NULL);
    wprintf(L"dir=%ls  (CreateFile2 + FILE_FLAG_OVERLAPPED)\n\n", dir);
    const wchar_t* names[] = { L"CON", L"CON.txt", L"NUL", L"NUL.txt", L"AUX", L"AUX.txt",
                               L"PRN", L"PRN.md", L"COM1", L"COM1.txt", L"COM10", L"COM10.log",
                               L"CONIN$", L"CONIN$.log", L"CONOUT$", L"CONOUT$.log", L"plain.txt" };
    for (unsigned i = 0; i < sizeof(names) / sizeof(names[0]); i++) probe2(dir, names[i], TRUE);
    return 0;
}