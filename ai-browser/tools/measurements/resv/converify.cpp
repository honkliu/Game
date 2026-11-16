// Definitive test: for names reported as "existing", read the bytes back and
// compare with what was written, and cross-check with FindFirstFile (directory
// enumeration), which only lists real directory entries.
#include <windows.h>
#include <stdio.h>
#include <string.h>
#include <stdlib.h>

static int read_back(const wchar_t* path, char* buf, DWORD cap) {
    HANDLE h = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, 0, NULL);
    if (h == INVALID_HANDLE_VALUE) return -1;
    DWORD got = 0;
    ReadFile(h, buf, cap - 1, &got, NULL);
    buf[got] = 0;
    CloseHandle(h);
    return (int)got;
}

static int listed_by_enumeration(const wchar_t* dir, const wchar_t* name) {
    wchar_t pat[MAX_PATH];
    swprintf(pat, MAX_PATH, L"%s\\*", dir);
    WIN32_FIND_DATAW fd;
    HANDLE f = FindFirstFileW(pat, &fd);
    if (f == INVALID_HANDLE_VALUE) return 0;
    int found = 0;
    do { if (_wcsicmp(fd.cFileName, name) == 0) { found = 1; break; } } while (FindNextFileW(f, &fd));
    FindClose(f);
    return found;
}

int main(void) {
    wchar_t dir[MAX_PATH];
    GetTempPathW(MAX_PATH, dir);
    wcscat_s(dir, MAX_PATH, L"converify");
    CreateDirectoryW(dir, NULL);
    wprintf(L"dir=%ls\n", dir);
    wprintf(L"%-12s %-6s %-8s %-10s %-8s %s\n", "name", "open", "write", "readback", "enum", "verdict");
    const wchar_t* names[] = { L"CON", L"CON.txt", L"NUL", L"NUL.txt", L"AUX.txt",
                               L"PRN.md", L"COM1", L"COM10.log", L"CONIN$.log", L"plain.txt" };
    for (unsigned i = 0; i < sizeof(names) / sizeof(names[0]); i++) {
        wchar_t path[MAX_PATH];
        swprintf(path, MAX_PATH, L"%s\\%s", dir, names[i]);
        HANDLE h = CreateFileW(path, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
        int opened = h != INVALID_HANDLE_VALUE;
        DWORD wrote = 0;
        if (opened) { WriteFile(h, "abc", 3, &wrote, NULL); CloseHandle(h); }
        char buf[16];
        int rd = read_back(path, buf, sizeof buf);
        int listed = listed_by_enumeration(dir, names[i]);
        const char* verdict = (!listed) ? "NOT A FILE" : (rd == 3 && strcmp(buf, "abc") == 0) ? "real file" : "something else";
        wprintf(L"%-12s %-6ls %-8lu %-10d %-8d %hs\n", names[i],
                opened ? L"ok" : L"FAIL", wrote, rd, listed, verdict);
        if (listed) DeleteFileW(path);
    }
    return 0;
}