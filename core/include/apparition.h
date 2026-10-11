// Apparition core C ABI (core/src/lib.rs). Every call blocks; call off the main thread.
// Strings returned by apparition_* calls are JSON and must be freed with apparition_string_free.
#ifndef APPARITION_H
#define APPARITION_H

#include <stdbool.h>

struct AdapterHandle;
struct RsdHandshakeHandle;

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

#endif
