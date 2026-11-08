#include <aria2/aria2.h>
#include "../vendor/gdirect_route.h"
#include <chrono>
#include <cstdint>
#include <cstdio>
#include <string>

// One synchronous session per call. No C++ exception crosses the C ABI.
// libaria2 is initialised once per process and never deinitialised: aria2's
// Platform::tearDown() frees the global option registry without resetting it,
// so a second libraryInit()/sessionNew() would read freed memory and crash.
// Callers serialise sessions (download::SESSION), so the flag needs no atomics.
static int library_state = 0;  // 0 = not yet, 1 = ready, -1 = failed
static bool library_ready() {
  if (library_state == 0) library_state = aria2::libraryInit() == 0 ? 1 : -1;
  return library_state == 1;
}

using ProgressCallback = int (*)(int64_t, int64_t, int, int, void*);
extern "C" int gdirect_download_ex(const char* url, const char* dir, const char* name,
                                const char* ip, const char* sni, const char* token,
                                const char* ca, int connections, ProgressCallback callback, void* data, bool signals) noexcept {
  aria2::Session* session = nullptr;
  int result = -1;
  try {
    gdirect_ip = ip;
    gdirect_sni = sni;
    if (!library_ready()) { gdirect_ip = gdirect_sni = nullptr; return -1; }
    aria2::KeyVals opts = {
      {"dir", dir}, {"out", name}, {"continue", "true"},
      {"auto-file-renaming", "false"}, {"allow-overwrite", "false"},
      {"async-dns", "false"}, {"check-certificate", "true"},
      {"http-proxy", ""}, {"https-proxy", ""}, {"all-proxy", ""},
      {"no-proxy", "*"}, {"enable-rpc", "false"},
      {"split", std::to_string(connections)},
      {"max-connection-per-server", std::to_string(connections)},
      {"min-split-size", "1M"}, {"connect-timeout", "8"}, {"timeout", "20"},
      {"max-tries", "2"}, {"retry-wait", "1"},
      {"file-allocation", "none"}, {"auto-save-interval", "5"},
      {"console-log-level", "error"}, {"summary-interval", "0"},
      {"enable-color", "false"}, {"quiet", "true"}
    };
    if (token && *token) opts.emplace_back("header", std::string("Authorization: Bearer ") + token);
    if (ca && *ca) opts.emplace_back("ca-certificate", ca);
    aria2::SessionConfig config;
    config.useSignalHandler = signals;
    session = aria2::sessionNew(opts, config);
    if (session) {
      aria2::A2Gid gid;
      if (aria2::addUri(session, &gid, {url}, opts) == 0) {
        auto last = std::chrono::steady_clock::now();
        while (aria2::run(session, aria2::RUN_ONCE) == 1) {
          auto now = std::chrono::steady_clock::now();
          if (now-last >= std::chrono::milliseconds(100)) {
            auto stat = aria2::getGlobalStat(session);
            if (callback) {
              auto* progress = aria2::getDownloadHandle(session, gid);
              if (progress) {
                const int cancel = callback(progress->getCompletedLength(), progress->getTotalLength(),
                  stat.downloadSpeed, progress->getConnections(), data);
                aria2::deleteDownloadHandle(progress);
                if (cancel) aria2::shutdown(session, true);
              }
            } else {
              std::fprintf(stderr, "\rDownloading: %d KiB/s", stat.downloadSpeed / 1024);
            }
            last = now;
          }
        }
        auto* handle = aria2::getDownloadHandle(session, gid);
        if (handle) {
          result = handle->getStatus() == aria2::DOWNLOAD_COMPLETE ? 0 : handle->getErrorCode();
          if (result == 0 && handle->getStatus() != aria2::DOWNLOAD_COMPLETE) result = -2;
          aria2::deleteDownloadHandle(handle);
        }
      }
      aria2::sessionFinal(session);
      session = nullptr;
    }
  } catch (...) {
    result = -1;
    if (session) { try { aria2::sessionFinal(session); } catch (...) {} }
  }
  gdirect_ip = gdirect_sni = nullptr;
  if (!callback) std::fprintf(stderr, "\n");
  return result;
}

extern "C" int gdirect_download(const char* url, const char* dir, const char* name,
                                const char* ip, const char* sni, const char* token,
                                const char* ca, int connections) noexcept {
  return gdirect_download_ex(url, dir, name, ip, sni, token, ca, connections, nullptr, nullptr, true);
}
