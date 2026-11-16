// The failure that matters for a browser: when the process has an interactive
// console attached, does CreateFileW("CON.txt") divert the bytes to the console
// instead of creating a file? Run this BOTH with a console and (by preference)
// detached, and compare.
#include <windows.h>
#include <stdio.h>

static void probe(const wchar_t* dir, const wchar_t* name) {
    wchar_t path[MAX_PATH];
    swprintf(path, MAX_PATH, L"%s\\%s", dir, name);
    HANDLE h = CreateFileW(path, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
    DWORD wrote = 0;
    if (h != INVALID_HANDLE_VALUE) {
        // Detect whether the handle is the console, not a file.
        DWORD type = GetFileType(h);
        WriteFile(h, "PAYLOAD", 7, &wrote, NULL);
        CloseHandle(h);
        WIN32_FIND_DATAW fd;
        HANDLE f = FindFirstFileW(path, &fd);
        int listed = f != INVALID_HANDLE_VALUE;
        DWORD size = listed ? fd.nFileSizeLow : 0;
        if (f != INVALID_HANDLE_VALUE) FindClose(f);
        wprintf(L"%-12s handle_type=%-6lu listed=%d size=%lu wrote=%lu\n",
                name, type, listed, size, wrote);
        if (listed) DeleteFileW(path);
    } else {
        wprintf(L"%-12s CreateFile FAILED err=%lu\n", name, GetLastError());
    }
}

int main(void) {
    wchar_t dir[MAX_PATH];
    GetTempPathW(MAX_PATH, dir);
    wcscat_s(dir, MAX_PATH, L"consdetect");
    CreateDirectoryW(dir, NULL);
    // GetStdHandle tells us whether we have a console at all.
    HANDLE out = GetStdHandle(STD_OUTPUT_HANDLE);
    DWORD outType = out && out != INVALID_HANDLE_VALUE ? GetFileType(out) : 0;
    DWORD session = 0;
    ProcessIdToSessionId(GetCurrentProcessId(), &session);
    fprintf(stderr, "stdout_type=%lu (2=char/console,1=disk,3=pipe) session=%lu\n", outType, session);
    fflush(stderr);
    const wchar_t* names[] = { L"CON", L"CON.txt", L"AUX.txt", L"PRN.md", L"CONIN$.log", L"NUL", L"plain.txt" };
    for (unsigned i = 0; i < sizeof(names) / sizeof(names[0]); i++) probe(dir, names[i]);
    return 0;
}