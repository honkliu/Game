// What DOES create a CON file? cmd's "> file" did; CreateFileW with
// FILE_ATTRIBUTE_NORMAL did too in converify. Test the variants that matter for
// a browser writing a downloaded file, and test whether the console steals the
// bytes (the dangerous failure: success reported, data lost).
#include <windows.h>
#include <stdio.h>

static void probe(const wchar_t* name, DWORD flags, DWORD disposition, const wchar_t* dir) {
    wchar_t path[MAX_PATH];
    swprintf(path, MAX_PATH, L"%s\\%s", dir, name);
    HANDLE h = CreateFileW(path, GENERIC_WRITE, 0, NULL, disposition, flags, NULL);
    DWORD err = GetLastError();
    DWORD wrote = 0;
    if (h != INVALID_HANDLE_VALUE) { WriteFile(h, "PAYLOAD", 7, &wrote, NULL); CloseHandle(h); }
    // Does a directory entry exist?
    wchar_t pat[MAX_PATH];
    swprintf(pat, MAX_PATH, L"%s\\%s", dir, name);
    WIN32_FIND_DATAW fd;
    HANDLE f = FindFirstFileW(pat, &fd);
    int listed = f != INVALID_HANDLE_VALUE;
    DWORD size = listed ? fd.nFileSizeLow : 0;
    if (f != INVALID_HANDLE_VALUE) FindClose(f);
    wprintf(L"%-10s flags=0x%04lX disp=%lu wrote=%lu listed=%d size=%lu\n",
            name, flags, disposition, wrote, listed, size);
    if (listed) DeleteFileW(path);
}

int main(void) {
    wchar_t dir[MAX_PATH];
    GetTempPathW(MAX_PATH, dir);
    wcscat_s(dir, MAX_PATH, L"conflags");
    CreateDirectoryW(dir, NULL);
    wprintf(L"dir=%ls\n", dir);
    wprintf(L"\nCreateFileW variants for CON (extensionless) and CON.txt:\n");
    for (int i = 0; i < 2; i++) {
        const wchar_t* n = i == 0 ? L"CON" : L"CON.txt";
        probe(n, FILE_ATTRIBUTE_NORMAL, CREATE_ALWAYS, dir);
        probe(n, FILE_ATTRIBUTE_NORMAL | FILE_FLAG_WRITE_THROUGH, CREATE_ALWAYS, dir);
        probe(n, 0, CREATE_ALWAYS, dir);
        probe(n, FILE_ATTRIBUTE_NORMAL, CREATE_NEW, dir);
    }
    wprintf(L"\nNUL with size check (does it swallow bytes?):\n");
    probe(L"NUL", FILE_ATTRIBUTE_NORMAL, CREATE_ALWAYS, dir);
    probe(L"NUL.txt", FILE_ATTRIBUTE_NORMAL, CREATE_ALWAYS, dir);
    return 0;
}