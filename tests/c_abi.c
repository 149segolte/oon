/* SPDX-License-Identifier: MPL-2.0 */

#include "oon.h"

#include <assert.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static const char SCHEMA_TEXT[] =
    "schema config = { value = int; label = string; };";
static const char OVERLAY_TEXT[] =
    "schema = \"CONFIG\"; overlay test = { .value = 7; .label = \"ok\"; };";

typedef struct ValueContext {
    OonValue *value;
} ValueContext;

static OonSource source(const char *name, const char *text) {
    OonSource result = {
        .name = (const uint8_t *)name,
        .name_len = strlen(name),
        .text = (const uint8_t *)text,
        .text_len = strlen(text),
    };
    return result;
}

static void *evaluate_one(void *unused) {
    (void)unused;
    OonSource schema = source("schema", SCHEMA_TEXT);
    OonSource overlay = source("overlay", OVERLAY_TEXT);
    OonOutput output = oon_evaluate_v1(&schema, &overlay, 1);
    assert(output.status == 0);
    assert(output.bytes != NULL);
    char *copy = malloc(output.len + 1);
    assert(copy != NULL);
    memcpy(copy, output.bytes, output.len);
    copy[output.len] = '\0';
    assert(strstr(copy, "\"value\": 7") != NULL);
    assert(copy[output.len - 1] == '\n');
    free(copy);
    oon_output_free(output);
    return NULL;
}

static void *evaluate_value_one(void *context_pointer) {
    ValueContext *context = context_pointer;
    OonSource schema = source("schema", SCHEMA_TEXT);
    OonSource overlay = source("overlay", OVERLAY_TEXT);
    OonOutput output =
        oon_evaluate_value_v1(&schema, context->value, &overlay, 1);
    assert(output.status == 0);
    assert(output.len != 0);
    assert(output.bytes[output.len - 1] == '\n');
    oon_output_free(output);
    return NULL;
}

int main(void) {
    assert(oon_abi_version() == 1);
    pthread_t threads[8];
    for (size_t index = 0; index < 8; ++index) {
        assert(pthread_create(&threads[index], NULL, evaluate_one, NULL) == 0);
    }
    for (size_t index = 0; index < 8; ++index) {
        assert(pthread_join(threads[index], NULL) == 0);
    }

    OonSource schema = source("schema", SCHEMA_TEXT);
    OonSource wrong = source("wrong", "schema = \"other\";");
    OonOutput diagnostic = oon_evaluate_v1(&schema, &wrong, 1);
    assert(diagnostic.status == 1);
    assert(diagnostic.len != 0);
    oon_output_free(diagnostic);

    const uint8_t malformed[] = {0xff};
    OonSource invalid = {
        .name = (const uint8_t *)"bad",
        .name_len = 3,
        .text = malformed,
        .text_len = sizeof(malformed),
    };
    OonOutput invalid_output = oon_evaluate_v1(&invalid, NULL, 0);
    assert(invalid_output.status == 2);
    oon_output_free(invalid_output);

    OonSource json = source("initial.json", "{\"value\":10,\"label\":\"start\"}");
    OonValue *value = NULL;
    OonOutput parsed = oon_value_from_json_v1(&json, &value);
    assert(parsed.status == 0);
    assert(parsed.bytes == NULL);
    assert(parsed.len == 0);
    assert(value != NULL);
    oon_output_free(parsed);

    ValueContext context = {.value = value};
    for (size_t index = 0; index < 8; ++index) {
        assert(pthread_create(&threads[index], NULL, evaluate_value_one,
                              &context) == 0);
    }
    for (size_t index = 0; index < 8; ++index) {
        assert(pthread_join(threads[index], NULL) == 0);
    }

    OonSource malformed_json = source("bad.json", "{");
    OonValue *malformed_value = (OonValue *)1;
    OonOutput malformed_result =
        oon_value_from_json_v1(&malformed_json, &malformed_value);
    assert(malformed_result.status == 1);
    assert(malformed_value == NULL);
    oon_output_free(malformed_result);

    OonSource wrong_json =
        source("wrong.json", "{\"value\":\"bad\",\"label\":\"ok\"}");
    OonValue *wrong_value = NULL;
    OonOutput wrong_parsed = oon_value_from_json_v1(&wrong_json, &wrong_value);
    assert(wrong_parsed.status == 0);
    oon_output_free(wrong_parsed);
    OonOutput wrong_evaluation =
        oon_evaluate_value_v1(&schema, wrong_value, NULL, 0);
    assert(wrong_evaluation.status == 1);
    oon_output_free(wrong_evaluation);

    OonOutput null_value = oon_evaluate_value_v1(&schema, NULL, NULL, 0);
    assert(null_value.status == 2);
    oon_output_free(null_value);
    OonOutput null_out = oon_value_from_json_v1(&json, NULL);
    assert(null_out.status == 2);
    oon_output_free(null_out);

    oon_value_free(wrong_value);
    oon_value_free(value);
    oon_value_free(NULL);
    return 0;
}
