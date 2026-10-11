// Apparition core C ABI (core/src/lib.rs). Every call blocks; call off the main thread.
// Strings returned by apparition_* calls are JSON and must be freed with apparition_string_free.
#ifndef APPARITION_H
#define APPARITION_H

#include <stdbool.h>

struct AdapterHandle;
struct RsdHandshakeHandle;
struct RpPairingFileHandle;

typedef const char *(*apparition_two_factor_fn)(void *context, const char *params_json);

void apparition_string_free(char *string);
void apparition_init_logging(const char *log_path);

char *apparition_login(const char *email, const char *password, const char *anisette_url,
                   const char *data_dir, apparition_two_factor_fn two_factor, void *context);
char *apparition_session(void);
char *apparition_sign(const char *app_path, const char *udid, const char *device_name,
                  bool strip_extensions);
// Handles from idevice's tunnel_create_rppairing; borrowed, not freed.
char *apparition_install(struct AdapterHandle *adapter, struct RsdHandshakeHandle *handshake,
                     const char *app_path);
// Launches bundle_id suspended, attaches debugserver over the tunnel and serves the
// brk #0x69 (UTM) and brk #0xf00d (JIT26) protocols, waiting up to wait_seconds.
char *apparition_launch_jit(struct AdapterHandle *adapter, struct RsdHandshakeHandle *handshake,
                        const char *bundle_id, unsigned int wait_seconds);

// Opens the RemotePairing tunnel at device_host:device_port, or, when relay ("ip:port") is
// non-null, through an Apparition Companion that connects back to this device (device_host is
// then ignored). Fills the same handles as idevice's tunnel_create_rppairing; free them with
// adapter_free / rsd_handshake_free. Returns {ok} or {ok: false, error}.
char *apparition_tunnel_create(const char *device_host, unsigned short device_port, const char *relay,
                           const char *hostname, struct RpPairingFileHandle *pairing_file,
                           const char *(*pin)(void *context), void *pin_context,
                           struct AdapterHandle **out_adapter, struct RsdHandshakeHandle **out_handshake);

// Pair-setup (or pair-verify) with the RemotePairing service on `port` through the companion at
// relay ("ip:port"), without a tunnel; updates pairing_file in place. Returns {ok} or {ok: false, error}.
char *apparition_pair_relay(const char *relay, unsigned short port, const char *hostname,
                            struct RpPairingFileHandle *pairing_file,
                            const char *(*pin)(void *context), void *pin_context);

// The account's development certificates and App IDs (core/src/account.rs). Revoking a
// certificate stops apps signed with it from launching; ask the user first.
char *apparition_list_certificates(void);
char *apparition_revoke_certificate(const char *serial_number);
char *apparition_list_app_ids(void);
char *apparition_delete_app_id(const char *app_id_id);

#endif
