/* Copyright (c) Rove contributors. Licensed under the MIT license. */
#include "tree_sitter/parser.h"

enum TokenType { CG_BODY, HLSL_BODY };

void *tree_sitter_shaderlab_external_scanner_create(void) { return NULL; }
void tree_sitter_shaderlab_external_scanner_destroy(void *payload) { (void)payload; }
unsigned tree_sitter_shaderlab_external_scanner_serialize(void *payload, char *buffer) {
  (void)payload;
  (void)buffer;
  return 0;
}
void tree_sitter_shaderlab_external_scanner_deserialize(void *payload, const char *buffer,
                                                       unsigned length) {
  (void)payload;
  (void)buffer;
  (void)length;
}

static bool line_ending(int32_t ch) { return ch == '\n' || ch == '\r'; }

static bool scan_terminator(TSLexer *lexer, const char *keyword, bool *consumed) {
  while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
    lexer->advance(lexer, false);
    *consumed = true;
  }
  for (; *keyword; ++keyword) {
    int32_t ch = lexer->lookahead;
    if (ch >= 'a' && ch <= 'z') ch -= 'a' - 'A';
    if (ch != *keyword) return false;
    lexer->advance(lexer, false);
    *consumed = true;
  }
  int32_t next = lexer->lookahead;
  return lexer->eof(lexer) || line_ending(next) || next == ' ' || next == '\t' ||
         next == '/' || next == '}';
}

bool tree_sitter_shaderlab_external_scanner_scan(void *payload, TSLexer *lexer,
                                                const bool *valid_symbols) {
  (void)payload;
  /* Both tokens are valid in error recovery. Leave recovery to the grammar. */
  if (valid_symbols[CG_BODY] == valid_symbols[HLSL_BODY]) return false;
  enum TokenType token = valid_symbols[CG_BODY] ? CG_BODY : HLSL_BODY;
  const char *terminator = token == CG_BODY ? "ENDCG" : "ENDHLSL";
  bool line_start = true;
  bool has_body = false;

  while (!lexer->eof(lexer)) {
    if (line_start) {
      lexer->mark_end(lexer);
      bool prefix = false;
      if (scan_terminator(lexer, terminator, &prefix)) {
        if (!has_body) return false;
        lexer->result_symbol = token;
        return true;
      }
      has_body |= prefix;
      if (lexer->eof(lexer)) break;
    }
    line_start = line_ending(lexer->lookahead);
    lexer->advance(lexer, false);
    has_body = true;
  }

  if (!has_body) return false;
  lexer->mark_end(lexer);
  lexer->result_symbol = token;
  return true;
}
