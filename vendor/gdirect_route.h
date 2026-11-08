// gdirect's narrow routing hook for the pinned aria2 1.37.0 build.
#pragma once
#include <cstring>
#include <string>
extern const char* gdirect_ip;
// Empty means no override. "" as a set value means "send no SNI"; a hostname
// means the forged identity: connect and verify as that name while the HTTP
// Host header carries the real Google host, which each Request keeps.
extern const char* gdirect_sni;
// Index of the physical interface to bind connections to (IP_BOUND_IF on
// macOS, SO_BINDTOIFINDEX on Linux, IP_UNICAST_IF on Windows), so a TUN proxy
// cannot capture them; 0 leaves them unbound. Applies only while gdirect_ip
// pins the endpoint.
extern unsigned gdirect_ifindex;
inline bool gdirect_host(const std::string& h) {
  const std::string suffix = ".googleusercontent.com";
  return h == "drive.google.com" || h == "drive.usercontent.google.com" ||
         h == "www.googleapis.com" || h == "content.googleapis.com" ||
         h == "drive.googleapis.com" ||
         (h.size() > suffix.size() && h.compare(h.size()-suffix.size(), suffix.size(), suffix) == 0);
}
