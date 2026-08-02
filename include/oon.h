/* SPDX-License-Identifier: MPL-2.0 */

#ifndef OON_H
#define OON_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct OonSource {
    const uint8_t *name;
    size_t name_len;
    const uint8_t *text;
    size_t text_len;
} OonSource;

typedef struct OonOutput {
    uint32_t status;
    uint8_t *bytes;
    size_t len;
} OonOutput;

typedef struct OonValue OonValue;

uint32_t oon_abi_version(void);
OonOutput oon_evaluate_v1(const OonSource *schema,
                          const OonSource *overlays,
                          size_t count);
OonOutput oon_value_from_json_v1(const OonSource *json,
                                 OonValue **out_value);
OonOutput oon_evaluate_value_v1(const OonSource *schema,
                                const OonValue *value,
                                const OonSource *overlays,
                                size_t count);
void oon_value_free(OonValue *value);
void oon_output_free(OonOutput output);

#ifdef __cplusplus
}
#endif

#endif
