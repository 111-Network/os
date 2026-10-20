/*
 * harness.c -- Layer 2 (identity) host-side Monocypher C cross-vector harness (T9).
 *
 * Spec: os/2-identity/README.md v2.1 (v2 LOCKED + 2026-10-14 section 3 amendment), section 14 ("Rust <-> Monocypher-C
 * agreement vectors REQUIRED before any Cat-1 port") and section 11 / D14 (this
 * run doubles as Monocypher's rubric smoke-test log).
 *
 * What it does: re-executes the frozen test vectors in os/2-identity/test-vectors/
 * (produced by the Rust id111 crate) against the vendored Monocypher 4.0.3
 * C library and demands byte-exact agreement:
 *
 *   ed25519-rfc8032.json        keygen-from-seed, sign (deterministic), verify
 *   x25519-rfc7748.json         ECDH: RFC 7748 5.2 / 6.1 / iterated
 *   slip0010-published.json     SLIP-0010 (HMAC-SHA512) published vectors
 *   slip0010-project-paths.json BIP-39 seed (PBKDF2) + m/111'/0'/{0',1'}
 *   keyset-project.json         mnemonic -> NodeId + X25519 pub (full chain)
 *   edge-vectors.json           the section 3 STRICT predicate (the oracle)
 *   announce.json               tag-prefixed announce verify + ingress order
 *   sealed-packet.json          section 6.4 recipe: X25519 + BLAKE2b KDFs +
 *                               XChaCha20-Poly1305, seal AND open
 *   attestation.json            section 10 node_sig under "111/attest-node/v1"
 *
 * Design decisions (documented, judgment calls):
 *
 *  1. Ed25519 here is RFC 8032 Ed25519 (SHA-512), i.e. Monocypher's OPTIONAL
 *     monocypher-ed25519 module (crypto_ed25519_*). Monocypher's core
 *     crypto_eddsa_* is EdDSA-BLAKE2b and is NOT wire-compatible with dalek.
 *
 *  2. crypto_ed25519_check is a COFACTORED verifier that ALLOWS non-canonical
 *     A and R encodings (see monocypher.c crypto_eddsa_check_equation), so it
 *     is wrapped by strict_verify(), which applies the section 3 pre-checks in
 *     the SAME ORDER as id111's keyset::verify_strict, giving the same
 *     VerifyError variant names: canonical A, canonical R, S < l, A decodes,
 *     R decodes, small-order R/A rejected, A and R torsion-free ([l]P = O,
 *     spec v2.1), then the Monocypher equation. Raw (unwrapped) verdicts are
 *     recorded alongside to document the delta. The torsion-free leg closed
 *     the T9 F3 residual (2026-10-14): the equation stays cofactored, but with
 *     A and R in the prime-order subgroup a cofactored and a cofactorless
 *     equation accept exactly the same inputs, so MIXED-ORDER R or A
 *     (prime-order + torsion component, not small-order) is rejected here as
 *     by id111. Monocypher's public API cannot express [l]P, so the harness
 *     computes it with its own reference arithmetic (point_torsion_free).
 *     edge-vectors.json freezes four mixed-order rows.
 *
 *  3. Monocypher 4.x crypto_x25519 returns void and does NOT reject all-zero
 *     output (changelog 4.0.0). ecdh() therefore applies the section 3 ingress
 *     blacklist AND the RFC 7748 6.1 all-zero abort itself; raw behaviour is
 *     recorded in the delta report.
 *
 *  4. SLIP-0010 / BIP-39 seed derivation are done IN C with the vendored
 *     crypto_sha512_hmac (PBKDF2-HMAC-SHA512 written here, ~15 lines). The
 *     mnemonic is treated as an opaque ASCII string (NFKD is the identity for
 *     the public English test phrases); BIP-39 wordlist/checksum validation
 *     is Rust-side policy and out of Monocypher's scope.
 *
 *  5. JSON is read by a small strict reader in this file (objects, arrays,
 *     strings, numbers; rejects duplicate keys and trailing garbage, like the
 *     Rust conformance reader). No Python is involved at run time.
 *
 *  6. No vector may be silently skipped: every consumed array's length is
 *     compared with the number of entries executed, unknown expect tokens
 *     FAIL, and files not consumed are listed with a reason (--list lets
 *     run.sh assert that every *.json in test-vectors/ is accounted for).
 *
 * C99, libc + vendored Monocypher only. Exit status 0 iff every check passed.
 */
#include <stddef.h>
#include <stdint.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "monocypher.h"
#include "monocypher-ed25519.h"

/* ------------------------------------------------------------------------ */
/* misc utilities                                                            */
/* ------------------------------------------------------------------------ */

static void die(const char *fmt, ...)
{
	va_list ap;
	fputs("harness: fatal: ", stderr);
	va_start(ap, fmt);
	vfprintf(stderr, fmt, ap);
	va_end(ap);
	fputc('\n', stderr);
	exit(2);
}

/* Tracked allocations, released after each vector file (keeps ASan/LSan clean). */
static void **g_allocs;
static size_t g_nallocs, g_capallocs;

static void *xalloc(size_t n)
{
	void *p = calloc(n ? n : 1, 1);
	if (!p) die("out of memory");
	if (g_nallocs == g_capallocs) {
		size_t cap = g_capallocs ? g_capallocs * 2 : 256;
		void **q = realloc(g_allocs, cap * sizeof *q);
		if (!q) die("out of memory");
		g_allocs = q;
		g_capallocs = cap;
	}
	g_allocs[g_nallocs++] = p;
	return p;
}

static void xfree_all(void)
{
	for (size_t i = 0; i < g_nallocs; i++) free(g_allocs[i]);
	g_nallocs = 0;
}

static int hexval(int c)
{
	if (c >= '0' && c <= '9') return c - '0';
	if (c >= 'a' && c <= 'f') return c - 'a' + 10;
	return -1; /* the frozen files are lowercase hex only */
}

/* Decode lowercase hex into a tracked buffer. Returns length, or -1 if invalid. */
static long unhex(const char *s, uint8_t **out)
{
	size_t n = strlen(s);
	if (n % 2) return -1;
	uint8_t *b = xalloc(n / 2);
	for (size_t i = 0; i < n / 2; i++) {
		int hi = hexval((unsigned char)s[2 * i]);
		int lo = hexval((unsigned char)s[2 * i + 1]);
		if (hi < 0 || lo < 0) return -1;
		b[i] = (uint8_t)(hi << 4 | lo);
	}
	*out = b;
	return (long)(n / 2);
}

static char *tohex(const uint8_t *b, size_t n)
{
	static const char d[] = "0123456789abcdef";
	char *s = xalloc(2 * n + 1);
	for (size_t i = 0; i < n; i++) {
		s[2 * i] = d[b[i] >> 4];
		s[2 * i + 1] = d[b[i] & 15];
	}
	return s;
}

static int all_zero(const uint8_t *b, size_t n)
{
	uint8_t acc = 0;
	for (size_t i = 0; i < n; i++) acc |= b[i];
	return acc == 0;
}

static void store_le64(uint8_t o[8], uint64_t v)
{
	for (int i = 0; i < 8; i++) o[i] = (uint8_t)(v >> (8 * i));
}

static uint64_t load_le64(const uint8_t b[8])
{
	uint64_t v = 0;
	for (int i = 7; i >= 0; i--) v = v << 8 | b[i];
	return v;
}

/* ------------------------------------------------------------------------ */
/* result accounting                                                         */
/* ------------------------------------------------------------------------ */

typedef struct {
	const char *name;
	int vectors;          /* entries executed */
	int expected_vectors; /* entries present in the file (inventory) */
	int checks, pass, fail;
	int skipped;          /* entries deliberately not executed (documented) */
} fstat;

static fstat *cur;
static int g_verbose;

static void check_(int ok, const char *fmt, ...)
{
	char msg[512];
	va_list ap;
	va_start(ap, fmt);
	vsnprintf(msg, sizeof msg, fmt, ap);
	va_end(ap);
	cur->checks++;
	if (ok) {
		cur->pass++;
		if (g_verbose) printf("  ok   [%s] %s\n", cur->name, msg);
	} else {
		cur->fail++;
		printf("  FAIL [%s] %s\n", cur->name, msg);
	}
}
#define CHECK(ok, ...) check_((ok) ? 1 : 0, __VA_ARGS__)

/* got[0..n) must equal the hex string `want`. */
static void expect_hex(const char *ctx, const char *what, const uint8_t *got, size_t n, const char *want)
{
	uint8_t *w = NULL;
	long wn = unhex(want, &w);
	int ok = wn >= 0 && (size_t)wn == n && (n == 0 || memcmp(got, w, n) == 0);
	if (ok)
		check_(1, "%s: %s", ctx, what);
	else
		check_(0, "%s: %s\n         got  %s\n         want %s", ctx, what, tohex(got, n), want);
}

/* ------------------------------------------------------------------------ */
/* strict JSON reader                                                        */
/* ------------------------------------------------------------------------ */

typedef enum { J_NULL, J_BOOL, J_NUM, J_STR, J_ARR, J_OBJ } jtype;

typedef struct jv {
	jtype t;
	char *s;          /* J_STR: decoded bytes (NUL-terminated); J_NUM: source text */
	size_t n;         /* J_ARR/J_OBJ: member count; J_STR: byte length */
	long long i;      /* J_NUM (integers) / J_BOOL */
	struct jv **kid;  /* J_ARR / J_OBJ values */
	char **key;       /* J_OBJ keys */
} jv;

typedef struct {
	const char *p, *end;
	const char *err;
	int depth;
} jp;

static jv *jp_value(jp *P);

static void jp_ws(jp *P)
{
	while (P->p < P->end && (*P->p == ' ' || *P->p == '\t' || *P->p == '\n' || *P->p == '\r')) P->p++;
}

static jv *jp_new(jtype t)
{
	jv *v = xalloc(sizeof *v);
	v->t = t;
	return v;
}

static int utf8_put(char *o, unsigned long cp)
{
	if (cp < 0x80) { o[0] = (char)cp; return 1; }
	if (cp < 0x800) { o[0] = (char)(0xC0 | cp >> 6); o[1] = (char)(0x80 | (cp & 63)); return 2; }
	if (cp < 0x10000) {
		o[0] = (char)(0xE0 | cp >> 12); o[1] = (char)(0x80 | ((cp >> 6) & 63));
		o[2] = (char)(0x80 | (cp & 63)); return 3;
	}
	o[0] = (char)(0xF0 | cp >> 18); o[1] = (char)(0x80 | ((cp >> 12) & 63));
	o[2] = (char)(0x80 | ((cp >> 6) & 63)); o[3] = (char)(0x80 | (cp & 63)); return 4;
}

static int jp_hex4(const char *p, unsigned long *out)
{
	unsigned long v = 0;
	for (int i = 0; i < 4; i++) {
		int h = hexval((unsigned char)(p[i] | 0x20)); /* accept A-F */
		if (p[i] >= '0' && p[i] <= '9') h = p[i] - '0';
		if (h < 0) return 0;
		v = v << 4 | (unsigned long)h;
	}
	*out = v;
	return 1;
}

static char *jp_string(jp *P, size_t *outlen)
{
	/* *P->p == '"' */
	const char *q = ++P->p;
	while (q < P->end && *q != '"') {
		if ((unsigned char)*q < 0x20) { P->err = "control character in string"; return NULL; }
		if (*q == '\\') q++;
		q++;
	}
	if (q >= P->end) { P->err = "unterminated string"; return NULL; }
	char *buf = xalloc((size_t)(q - P->p) + 1);
	size_t o = 0;
	while (P->p < q) {
		char c = *P->p++;
		if (c != '\\') { buf[o++] = c; continue; }
		char e = *P->p++;
		switch (e) {
		case '"': buf[o++] = '"'; break;
		case '\\': buf[o++] = '\\'; break;
		case '/': buf[o++] = '/'; break;
		case 'b': buf[o++] = '\b'; break;
		case 'f': buf[o++] = '\f'; break;
		case 'n': buf[o++] = '\n'; break;
		case 'r': buf[o++] = '\r'; break;
		case 't': buf[o++] = '\t'; break;
		case 'u': {
			unsigned long cp, lo;
			if (q - P->p < 4 || !jp_hex4(P->p, &cp)) { P->err = "bad \\u escape"; return NULL; }
			P->p += 4;
			if (cp >= 0xD800 && cp < 0xDC00) { /* surrogate pair */
				if (q - P->p < 6 || P->p[0] != '\\' || P->p[1] != 'u' || !jp_hex4(P->p + 2, &lo) ||
				    lo < 0xDC00 || lo > 0xDFFF) {
					P->err = "bad surrogate pair";
					return NULL;
				}
				P->p += 6;
				cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
			}
			o += (size_t)utf8_put(buf + o, cp);
			break;
		}
		default: P->err = "bad escape"; return NULL;
		}
	}
	P->p = q + 1;
	buf[o] = 0;
	*outlen = o;
	return buf;
}

static jv *jp_array(jp *P)
{
	jv *v = jp_new(J_ARR);
	jv **tmp = NULL;
	size_t cnt = 0, cap = 0;
	P->p++; /* [ */
	jp_ws(P);
	if (P->p < P->end && *P->p == ']') { P->p++; return v; }
	for (;;) {
		jv *e = jp_value(P);
		if (!e) { free(tmp); return NULL; }
		if (cnt == cap) {
			cap = cap ? cap * 2 : 8;
			jv **t2 = realloc(tmp, cap * sizeof *t2);
			if (!t2) die("out of memory");
			tmp = t2;
		}
		tmp[cnt++] = e;
		jp_ws(P);
		if (P->p < P->end && *P->p == ',') { P->p++; continue; }
		if (P->p < P->end && *P->p == ']') { P->p++; break; }
		P->err = "expected , or ] in array";
		free(tmp);
		return NULL;
	}
	v->n = cnt;
	v->kid = xalloc(cnt * sizeof *v->kid);
	memcpy(v->kid, tmp, cnt * sizeof *tmp);
	free(tmp);
	return v;
}

static jv *jp_object(jp *P)
{
	jv *v = jp_new(J_OBJ);
	jv **tv = NULL;
	char **tk = NULL;
	size_t cnt = 0, cap = 0;
	P->p++; /* { */
	jp_ws(P);
	if (P->p < P->end && *P->p == '}') { P->p++; return v; }
	for (;;) {
		jp_ws(P);
		if (P->p >= P->end || *P->p != '"') { P->err = "expected string key"; goto fail; }
		size_t kl;
		char *k = jp_string(P, &kl);
		if (!k) goto fail;
		for (size_t i = 0; i < cnt; i++)
			if (strcmp(tk[i], k) == 0) { P->err = "duplicate object key"; goto fail; }
		jp_ws(P);
		if (P->p >= P->end || *P->p != ':') { P->err = "expected :"; goto fail; }
		P->p++;
		jv *e = jp_value(P);
		if (!e) goto fail;
		if (cnt == cap) {
			cap = cap ? cap * 2 : 8;
			jv **a = realloc(tv, cap * sizeof *a);
			char **b = realloc(tk, cap * sizeof *b);
			if (!a || !b) die("out of memory");
			tv = a;
			tk = b;
		}
		tv[cnt] = e;
		tk[cnt] = k;
		cnt++;
		jp_ws(P);
		if (P->p < P->end && *P->p == ',') { P->p++; continue; }
		if (P->p < P->end && *P->p == '}') { P->p++; break; }
		P->err = "expected , or } in object";
		goto fail;
	}
	v->n = cnt;
	v->kid = xalloc(cnt * sizeof *v->kid);
	v->key = xalloc(cnt * sizeof *v->key);
	memcpy(v->kid, tv, cnt * sizeof *tv);
	memcpy(v->key, tk, cnt * sizeof *tk);
	free(tv);
	free(tk);
	return v;
fail:
	free(tv);
	free(tk);
	return NULL;
}

static jv *jp_value(jp *P)
{
	jp_ws(P);
	if (P->p >= P->end) { P->err = "unexpected end of input"; return NULL; }
	if (++P->depth > 64) { P->err = "nesting too deep"; return NULL; }
	jv *v = NULL;
	char c = *P->p;
	if (c == '{') v = jp_object(P);
	else if (c == '[') v = jp_array(P);
	else if (c == '"') {
		size_t n;
		char *s = jp_string(P, &n);
		if (s) { v = jp_new(J_STR); v->s = s; v->n = n; }
	} else if (c == '-' || (c >= '0' && c <= '9')) {
		const char *q = P->p;
		int isint = 1;
		while (q < P->end && ((*q >= '0' && *q <= '9') || *q == '-' || *q == '+' || *q == '.' || *q == 'e' || *q == 'E')) {
			if (*q == '.' || *q == 'e' || *q == 'E') isint = 0;
			q++;
		}
		v = jp_new(J_NUM);
		v->s = xalloc((size_t)(q - P->p) + 1);
		memcpy(v->s, P->p, (size_t)(q - P->p));
		v->i = isint ? strtoll(v->s, NULL, 10) : 0;
		P->p = q;
	} else if (P->end - P->p >= 4 && memcmp(P->p, "true", 4) == 0) {
		v = jp_new(J_BOOL); v->i = 1; P->p += 4;
	} else if (P->end - P->p >= 5 && memcmp(P->p, "false", 5) == 0) {
		v = jp_new(J_BOOL); P->p += 5;
	} else if (P->end - P->p >= 4 && memcmp(P->p, "null", 4) == 0) {
		v = jp_new(J_NULL); P->p += 4;
	} else {
		P->err = "unexpected character";
	}
	P->depth--;
	return v;
}

static const char *g_dir = ".";

static const jv *load_json(const char *name)
{
	char path[1024];
	snprintf(path, sizeof path, "%s/%s", g_dir, name);
	FILE *f = fopen(path, "rb");
	if (!f) die("cannot open %s", path);
	size_t cap = 1 << 16, len = 0;
	char *buf = malloc(cap);
	if (!buf) die("out of memory");
	size_t r;
	while ((r = fread(buf + len, 1, cap - len, f)) > 0) {
		len += r;
		if (len == cap) {
			if (cap > (size_t)8 << 20) die("%s: too large", path);
			cap *= 2;
			char *b2 = realloc(buf, cap);
			if (!b2) die("out of memory");
			buf = b2;
		}
	}
	fclose(f);
	jp P = { buf, buf + len, NULL, 0 };
	jv *v = jp_value(&P);
	if (v) {
		jp_ws(&P);
		if (P.p != P.end) { v = NULL; P.err = "trailing garbage after JSON value"; }
	}
	if (!v) die("%s: JSON error at byte %ld: %s", path, (long)(P.p - buf), P.err ? P.err : "?");
	free(buf);
	return v;
}

static const jv *jopt(const jv *o, const char *k)
{
	if (o->t != J_OBJ) return NULL;
	for (size_t i = 0; i < o->n; i++)
		if (strcmp(o->key[i], k) == 0) return o->kid[i];
	return NULL;
}

static const jv *jfield(const jv *o, const char *k)
{
	const jv *v = jopt(o, k);
	if (!v) die("%s: missing field \"%s\"", cur ? cur->name : "?", k);
	return v;
}

static const char *jstr(const jv *o, const char *k)
{
	const jv *v = jfield(o, k);
	if (v->t != J_STR) die("%s: field \"%s\" is not a string", cur ? cur->name : "?", k);
	return v->s;
}

static long long jint(const jv *o, const char *k)
{
	const jv *v = jfield(o, k);
	if (v->t != J_NUM) die("%s: field \"%s\" is not a number", cur ? cur->name : "?", k);
	return v->i;
}

static const jv *jarr(const jv *o, const char *k)
{
	const jv *v = jfield(o, k);
	if (v->t != J_ARR) die("%s: field \"%s\" is not an array", cur ? cur->name : "?", k);
	return v;
}

static long jhex(const jv *o, const char *k, uint8_t **out)
{
	long n = unhex(jstr(o, k), out);
	if (n < 0) die("%s: field \"%s\" is not lowercase hex", cur ? cur->name : "?", k);
	return n;
}

static void jhexn(const jv *o, const char *k, uint8_t *out, size_t want)
{
	uint8_t *b;
	long n = jhex(o, k, &b);
	if ((size_t)n != want) die("%s: field \"%s\" is %ld bytes, want %zu", cur ? cur->name : "?", k, n, want);
	memcpy(out, b, want);
}

static uint64_t jdec64(const jv *o, const char *k) /* u64 as decimal string */
{
	return strtoull(jstr(o, k), NULL, 10);
}

/* ------------------------------------------------------------------------ */
/* constants (decoded from hex at startup; cross-checked against the vectors) */
/* ------------------------------------------------------------------------ */

static uint8_t FIELD_P[32];     /* 2^255 - 19, LE */
static uint8_t GROUP_L[32];     /* Ed25519 group order l, LE */
static uint8_t SMALL_Y[5][32];  /* the 5 distinct y-coords of the 8 small-order points */
static uint8_t XBL[7][32];      /* X25519 low-order u blacklist (libsodium class) */

static void hex32(uint8_t o[32], const char *h)
{
	uint8_t *b;
	if (unhex(h, &b) != 32) die("internal: bad constant %s", h);
	memcpy(o, b, 32);
}

static void init_tables(void)
{
	hex32(FIELD_P, "edffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f");
	hex32(GROUP_L, "edd3f55c1a631258d69cf7a2def9de1400000000000000000000000000000010");
	/* small-order points: y in { 1, p-1, 0, y8a, y8c }, each with both x signs */
	hex32(SMALL_Y[0], "0100000000000000000000000000000000000000000000000000000000000000");
	hex32(SMALL_Y[1], "ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f");
	hex32(SMALL_Y[2], "0000000000000000000000000000000000000000000000000000000000000000");
	hex32(SMALL_Y[3], "c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac037a");
	hex32(SMALL_Y[4], "26e8958fc2b227b045c3f489f2ef98f0d5dfac05d3c63339b13802886d53fc05");
	/* X25519: u = 0, 1, two order-8 points, p-1, p, p+1 (top bit masked first) */
	hex32(XBL[0], "0000000000000000000000000000000000000000000000000000000000000000");
	hex32(XBL[1], "0100000000000000000000000000000000000000000000000000000000000000");
	hex32(XBL[2], "e0eb7a7c3b41b8ae1656e3faf19fc46ada098deb9c32b1fd866205165f49b800");
	hex32(XBL[3], "5f9c95bca3508c24b1d0b1559c83ef5b04445cc4581c8e86d8224eddd09f1157");
	hex32(XBL[4], "ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f");
	hex32(XBL[5], "edffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f");
	hex32(XBL[6], "eeffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f");
}

/* ------------------------------------------------------------------------ */
/* section 3: the strict Ed25519 predicate wrapped around Monocypher         */
/* ------------------------------------------------------------------------ */

typedef enum {
	V_OK = 0,
	V_NONCANON_A,  /* NonCanonicalPublicKey */
	V_NONCANON_R,  /* NonCanonicalR */
	V_NONCANON_S,  /* NonCanonicalS */
	V_MALFORMED_A, /* MalformedPublicKey (A does not decode to a curve point) */
	V_INVALID      /* InvalidSignature (R undecodable, small-order A/R, bad equation) */
} verdict;

static const char *verdict_name(verdict v)
{
	switch (v) {
	case V_OK: return "ok";
	case V_NONCANON_A: return "NonCanonicalPublicKey";
	case V_NONCANON_R: return "NonCanonicalR";
	case V_NONCANON_S: return "NonCanonicalS";
	case V_MALFORMED_A: return "MalformedPublicKey";
	case V_INVALID: return "InvalidSignature";
	}
	return "?";
}

/* a < b as 256-bit little-endian integers (variable time; inputs are public) */
static int le_lt(const uint8_t a[32], const uint8_t b[32])
{
	for (int i = 31; i >= 0; i--)
		if (a[i] != b[i]) return a[i] < b[i];
	return 0;
}

/* y (bit 255 = x sign, masked) is a field element < 2^255 - 19 */
static int point_canonical(const uint8_t e[32])
{
	uint8_t y[32];
	memcpy(y, e, 32);
	y[31] &= 0x7f;
	return le_lt(y, FIELD_P);
}

/* Small-order <=> y in the 5-element table (x sign irrelevant). Precondition:
 * canonical encoding (a non-canonical alias must be rejected first). */
static int point_small_order(const uint8_t e[32])
{
	uint8_t y[32];
	memcpy(y, e, 32);
	y[31] &= 0x7f;
	for (int i = 0; i < 5; i++)
		if (crypto_verify32(y, SMALL_Y[i]) == 0) return 1;
	return 0;
}

/* Does `enc` decode to a curve point? Uses Monocypher's own decompression via
 * the public crypto_eddsa_check_equation with a probe that is accepted iff A
 * decodes: signature = (R = identity, S = 0), h = 0 => [8]([0]B - [0]A - O) = O. */
static int point_decodes(const uint8_t enc[32])
{
	static const uint8_t probe[64] = { 0x01 }; /* R = identity (y = 1), S = 0 */
	static const uint8_t h0[32] = { 0 };
	return crypto_eddsa_check_equation(probe, enc, h0) == 0;
}

/* ------------------------------------------------------------------------ */
/* section 3 v2.1: torsion-free leg, [l]P = O  -- HOST REFERENCE ONLY        */
/* ------------------------------------------------------------------------ */
/*
 * Monocypher 4.0.3's public API cannot compute [l]P for an arbitrary point:
 * crypto_eddsa_check_equation multiplies by the cofactor 8 before comparing
 * (a torsion component vanishes), crypto_eddsa_scalarbase multiplies only the
 * base point, crypto_eddsa_mul_add is scalar arithmetic, and the X25519 entry
 * points clamp the scalar to a multiple of 8 (torsion vanishes again). The
 * vendored sources are hash-pinned and not touched, so the harness carries
 * its own small Edwards arithmetic: TweetNaCl-shaped, 16 x 16-bit limbs in
 * int64, extended coordinates, the complete a = -1 addition law (also used
 * for doubling), plain double-and-add. Variable time and slow -- the inputs
 * are public points and the scalar is the public constant l -- so it is a
 * host reference for the agreement run, NOT the Cat-1 core's implementation
 * (which owns its own; spec section 3). Every carry/shift below is defined
 * behaviour (no left shift of negatives), so the --sanitize build covers it.
 * Cross-checked at start-up against Monocypher (h_selftest): [s]B computed
 * here must equal crypto_eddsa_scalarbase(s) byte-for-byte.
 */
typedef int64_t gf[16];

static gf GF_0, GF_1, ED_D, ED_D2, ED_SQRTM1; /* filled by torsion_tables() */

static void gf_set(gf o, const gf a)
{
	for (int i = 0; i < 16; i++) o[i] = a[i];
}

static void gf_unpack(gf o, const uint8_t e[32]) /* bit 255 (x sign) masked */
{
	for (int i = 0; i < 16; i++) o[i] = (int64_t)e[2 * i] + ((int64_t)e[2 * i + 1] << 8);
	o[15] &= 0x7fff;
}

static int64_t floor_div65536(int64_t x)
{
	return x >= 0 ? x / 65536 : -((-x - 1) / 65536) - 1;
}

/* one carry pass; 2^256 = 38 (mod p) folds the top carry into limb 0 */
static void gf_carry(gf o)
{
	for (int i = 0; i < 16; i++) {
		int64_t c = floor_div65536(o[i]);
		o[i] -= c * 65536;
		if (i < 15) o[i + 1] += c;
		else o[0] += 38 * c;
	}
}

static void gf_add(gf o, const gf a, const gf b)
{
	for (int i = 0; i < 16; i++) o[i] = a[i] + b[i];
}

static void gf_sub(gf o, const gf a, const gf b)
{
	for (int i = 0; i < 16; i++) o[i] = a[i] - b[i];
}

static void gf_mul(gf o, const gf a, const gf b)
{
	int64_t t[31] = { 0 };
	for (int i = 0; i < 16; i++)
		for (int j = 0; j < 16; j++) t[i + j] += a[i] * b[j];
	for (int i = 0; i < 15; i++) t[i] += 38 * t[i + 16];
	for (int i = 0; i < 16; i++) o[i] = t[i];
	gf_carry(o);
	gf_carry(o);
}

/* canonical little-endian bytes of n mod p */
static void gf_pack(uint8_t o[32], const gf n)
{
	gf t;
	gf_set(t, n);
	for (int k = 0; k < 4; k++) gf_carry(t);
	for (int i = 0; i < 16; i++)
		if (t[i] < 0 || t[i] > 0xffff) die("internal: [l]P reference: limb %d not normalised", i);
	for (int round = 0; round < 2; round++) { /* t < 2^256 < 3p: subtract p at most twice */
		gf m;
		int64_t borrow = 0;
		for (int i = 0; i < 16; i++) {
			int64_t pi = i == 0 ? 0xffed : i == 15 ? 0x7fff : 0xffff;
			m[i] = t[i] - pi - borrow;
			borrow = m[i] < 0;
			if (borrow) m[i] += 65536;
		}
		if (!borrow) gf_set(t, m);
	}
	for (int i = 0; i < 16; i++) {
		o[2 * i] = (uint8_t)(t[i] & 0xff);
		o[2 * i + 1] = (uint8_t)(t[i] >> 8);
	}
}

static int gf_eq(const gf a, const gf b)
{
	uint8_t x[32], y[32];
	gf_pack(x, a);
	gf_pack(y, b);
	return memcmp(x, y, 32) == 0;
}

static int gf_parity(const gf a)
{
	uint8_t x[32];
	gf_pack(x, a);
	return x[0] & 1;
}

static void gf_inv(gf o, const gf a) /* a^(p-2): p-2 = 2^255 - 21, bits 4 and 2 clear */
{
	gf c;
	gf_set(c, a);
	for (int i = 253; i >= 0; i--) {
		gf_mul(c, c, c);
		if (i != 2 && i != 4) gf_mul(c, c, a);
	}
	gf_set(o, c);
}

static void gf_pow2523(gf o, const gf a) /* a^((p-5)/8) = a^(2^252 - 3) */
{
	gf c;
	gf_set(c, a);
	for (int i = 250; i >= 0; i--) {
		gf_mul(c, c, c);
		if (i != 1) gf_mul(c, c, a);
	}
	gf_set(o, c);
}

static void torsion_tables(void)
{
	uint8_t b[32];
	memset(GF_0, 0, sizeof GF_0);
	memset(GF_1, 0, sizeof GF_1);
	GF_1[0] = 1;
	hex32(b, "a3785913ca4deb75abd841414d0a700098e879777940c78c73fe6f2bee6c0352"); /* d = -121665/121666 */
	gf_unpack(ED_D, b);
	hex32(b, "59f1b226949bd6eb56b183829a14e00030d1f3eef2808e19e7fcdf56dcd90624"); /* 2d */
	gf_unpack(ED_D2, b);
	hex32(b, "b0a00e4a271beec478e42fad0618432fa7d7fb3d99004d2b0bdfc14f8024832b"); /* sqrt(-1) */
	gf_unpack(ED_SQRTM1, b);
	/* the constants must satisfy their defining equations */
	gf t, u;
	gf_mul(t, ED_SQRTM1, ED_SQRTM1);
	gf_sub(u, GF_0, GF_1);
	if (!gf_eq(t, u)) die("internal: sqrt(-1) constant");
	gf_add(t, ED_D, ED_D);
	if (!gf_eq(t, ED_D2)) die("internal: 2d constant");
	gf c121666 = { 0 }, c121665 = { 0 };
	c121666[0] = 121666 & 0xffff, c121666[1] = 121666 >> 16;
	c121665[0] = 121665 & 0xffff, c121665[1] = 121665 >> 16;
	gf_mul(t, ED_D, c121666);
	gf_add(t, t, c121665);
	if (!gf_eq(t, GF_0)) die("internal: d constant");
}

/* extended coordinates (X : Y : Z : T), x = X/Z, y = Y/Z, xy = T/Z */
static void ge_identity(gf p[4])
{
	gf_set(p[0], GF_0);
	gf_set(p[1], GF_1);
	gf_set(p[2], GF_1);
	gf_set(p[3], GF_0);
}

/* p += q (complete law for a = -1, d non-square: valid for p == q and torsion) */
static void ge_add(gf p[4], gf q[4])
{
	gf a, b, c, d, t, e, f, g, h;
	gf_sub(a, p[1], p[0]);
	gf_sub(t, q[1], q[0]);
	gf_mul(a, a, t);
	gf_add(b, p[0], p[1]);
	gf_add(t, q[0], q[1]);
	gf_mul(b, b, t);
	gf_mul(c, p[3], q[3]);
	gf_mul(c, c, ED_D2);
	gf_mul(d, p[2], q[2]);
	gf_add(d, d, d);
	gf_sub(e, b, a);
	gf_sub(f, d, c);
	gf_add(g, d, c);
	gf_add(h, b, a);
	gf_mul(p[0], e, f);
	gf_mul(p[1], h, g);
	gf_mul(p[2], g, f);
	gf_mul(p[3], e, h);
}

/* r = [s]P, s a 256-bit little-endian integer (NOT reduced, NOT clamped) */
static void ge_scalarmult(gf r[4], gf P[4], const uint8_t s[32])
{
	ge_identity(r);
	for (int i = 255; i >= 0; i--) {
		ge_add(r, r);
		if ((s[i >> 3] >> (i & 7)) & 1) ge_add(r, P);
	}
}

/* RFC 8032 5.1.3 decoding (x sign from bit 255); -1 if y is not on the curve */
static int ge_decode(gf p[4], const uint8_t e[32])
{
	gf y, num, den, den2, den4, den6, t, chk, x;
	gf_unpack(y, e);
	gf_mul(num, y, y);
	gf_mul(den, num, ED_D);
	gf_sub(num, num, GF_1); /* u = y^2 - 1 */
	gf_add(den, den, GF_1); /* v = d y^2 + 1 */
	gf_mul(den2, den, den);
	gf_mul(den4, den2, den2);
	gf_mul(den6, den4, den2);
	gf_mul(t, den6, num);
	gf_mul(t, t, den); /* u v^7 */
	gf_pow2523(t, t);
	gf_mul(t, t, num);
	gf_mul(t, t, den);
	gf_mul(t, t, den);
	gf_mul(x, t, den); /* x = u v^3 (u v^7)^((p-5)/8) */
	gf_mul(chk, x, x);
	gf_mul(chk, chk, den);
	if (!gf_eq(chk, num)) gf_mul(x, x, ED_SQRTM1);
	gf_mul(chk, x, x);
	gf_mul(chk, chk, den);
	if (!gf_eq(chk, num)) return -1;
	if (gf_parity(x) != (e[31] >> 7)) gf_sub(x, GF_0, x);
	gf_set(p[0], x);
	gf_set(p[1], y);
	gf_set(p[2], GF_1);
	gf_mul(p[3], x, y);
	return 0;
}

static void ge_encode(uint8_t o[32], gf p[4])
{
	gf zi, x, y;
	gf_inv(zi, p[2]);
	gf_mul(x, p[0], zi);
	gf_mul(y, p[1], zi);
	gf_pack(o, y);
	o[31] = (uint8_t)(o[31] | (gf_parity(x) << 7));
}

/* Spec v2.1: the point is in the prime-order subgroup, [l]P = O. Precondition:
 * canonical and decodable (checked earlier); an own-decoder failure fails closed. */
static int point_torsion_free(const uint8_t e[32])
{
	static const uint8_t identity[32] = { 1 };
	gf P[4], Q[4];
	uint8_t q[32];
	if (ge_decode(P, e) != 0) return 0;
	ge_scalarmult(Q, P, GROUP_L);
	ge_encode(q, Q);
	return memcmp(q, identity, 32) == 0;
}

static verdict strict_verify(const uint8_t sig[64], const uint8_t pk[32], const uint8_t *msg, size_t n)
{
	if (!point_canonical(pk)) return V_NONCANON_A;
	if (!point_canonical(sig)) return V_NONCANON_R;
	if (!le_lt(sig + 32, GROUP_L)) return V_NONCANON_S;
	if (!point_decodes(pk)) return V_MALFORMED_A;
	if (!point_decodes(sig)) return V_INVALID; /* R not on the curve (dalek: Verify) */
	if (point_small_order(sig) || point_small_order(pk)) return V_INVALID;
	if (!point_torsion_free(pk)) return V_INVALID;  /* v2.1: mixed-order A (ghost NodeId A + T) */
	if (!point_torsion_free(sig)) return V_INVALID; /* v2.1: mixed-order R (malleability) */
	if (crypto_ed25519_check(sig, pk, msg, n) != 0) return V_INVALID;
	return V_OK;
}

/* tag || body, verified under the strict predicate (the "111/..." domain tags) */
static verdict strict_verify_tagged(const char *tag, const uint8_t *body, size_t bn, const uint8_t sig[64],
                                    const uint8_t pk[32])
{
	size_t tn = strlen(tag);
	uint8_t *m = xalloc(tn + bn);
	memcpy(m, tag, tn);
	memcpy(m + tn, body, bn);
	return strict_verify(sig, pk, m, tn + bn);
}

static void sign_tagged(uint8_t sig[64], const uint8_t sk[64], const char *tag, const uint8_t *body, size_t bn)
{
	size_t tn = strlen(tag);
	uint8_t *m = xalloc(tn + bn);
	memcpy(m, tag, tn);
	memcpy(m + tn, body, bn);
	crypto_ed25519_sign(sig, sk, m, tn + bn);
}

/* ------------------------------------------------------------------------ */
/* section 3: X25519 ingress check + all-zero abort around crypto_x25519     */
/* ------------------------------------------------------------------------ */

typedef enum { E_OK = 0, E_LOWORDER, E_NONCONTRIB } ecdh_err;

/* RFC 7748 6.1 / libsodium blacklist class; bit 255 is ignored by X25519. */
static int x25519_low_order(const uint8_t u[32])
{
	uint8_t m[32];
	memcpy(m, u, 32);
	m[31] &= 0x7f;
	for (int i = 0; i < 7; i++)
		if (crypto_verify32(m, XBL[i]) == 0) return 1;
	return 0;
}

static ecdh_err ecdh(uint8_t shared[32], const uint8_t sk[32], const uint8_t their_pk[32])
{
	if (x25519_low_order(their_pk)) return E_LOWORDER;
	crypto_x25519(shared, sk, their_pk);
	if (all_zero(shared, 32)) { /* RFC 7748 6.1: abort, never return the secret */
		crypto_wipe(shared, 32);
		return E_NONCONTRIB;
	}
	return E_OK;
}

/* ------------------------------------------------------------------------ */
/* BIP-39 seed (PBKDF2-HMAC-SHA512) and SLIP-0010 on the vendored HMAC       */
/* ------------------------------------------------------------------------ */

static void hmac512_2(uint8_t out[64], const uint8_t *key, size_t kn, const uint8_t *a, size_t an,
                      const uint8_t *b, size_t bn)
{
	crypto_sha512_hmac_ctx c;
	crypto_sha512_hmac_init(&c, key, kn);
	crypto_sha512_hmac_update(&c, a, an);
	if (bn) crypto_sha512_hmac_update(&c, b, bn);
	crypto_sha512_hmac_final(&c, out);
}

/* BIP-39: seed = PBKDF2-HMAC-SHA512(mnemonic, "mnemonic" || passphrase="", 2048, 64) */
static void bip39_seed(uint8_t seed[64], const char *mnemonic)
{
	static const uint8_t salt[12] = { 'm', 'n', 'e', 'm', 'o', 'n', 'i', 'c', 0, 0, 0, 1 }; /* || INT(1) */
	const uint8_t *pw = (const uint8_t *)mnemonic;
	size_t pn = strlen(mnemonic);
	uint8_t u[64], t[64];
	hmac512_2(u, pw, pn, salt, sizeof salt, NULL, 0);
	memcpy(t, u, 64);
	for (int i = 1; i < 2048; i++) {
		hmac512_2(u, pw, pn, u, 64, NULL, 0);
		for (int j = 0; j < 64; j++) t[j] ^= u[j];
	}
	memcpy(seed, t, 64);
}

/* SLIP-0010 ed25519, hardened-only. `idx` are RAW indices (hardened bit added
 * here). Returns 0, or -1 with *bad = the offending index (already >= 2^31). */
static int slip0010(const uint8_t *seed, size_t sn, const uint32_t *idx, size_t n, uint8_t key[32],
                    uint8_t chain[32], uint32_t *bad)
{
	static const uint8_t k0[12] = { 'e', 'd', '2', '5', '5', '1', '9', ' ', 's', 'e', 'e', 'd' };
	uint8_t I[64];
	hmac512_2(I, k0, sizeof k0, seed, sn, NULL, 0);
	for (size_t i = 0; i < n; i++) {
		if (idx[i] >= 0x80000000u) { *bad = idx[i]; return -1; }
		uint32_t h = idx[i] | 0x80000000u;
		uint8_t data[1 + 32 + 4];
		data[0] = 0;
		memcpy(data + 1, I, 32);
		data[33] = (uint8_t)(h >> 24);
		data[34] = (uint8_t)(h >> 16);
		data[35] = (uint8_t)(h >> 8);
		data[36] = (uint8_t)h;
		uint8_t J[64];
		hmac512_2(J, I + 32, 32, data, sizeof data, NULL, 0);
		memcpy(I, J, 64);
	}
	memcpy(key, I, 32);
	memcpy(chain, I + 32, 32);
	return 0;
}

typedef struct {
	uint8_t bip39[64];
	uint8_t sign_seed[32];
	uint8_t ecdh_scalar[32];
	uint8_t sk[64];  /* Monocypher Ed25519 secret key: seed || pubkey */
	uint8_t pk[32];  /* Ed25519 public key == NodeId (B2) */
	uint8_t xpk[32]; /* X25519 public key */
} keyset;

static void keyset_from_mnemonic(keyset *ks, const char *mnemonic)
{
	static const uint32_t sign_path[3] = { 111, 0, 0 };
	static const uint32_t ecdh_path[3] = { 111, 0, 1 };
	uint8_t chain[32];
	uint32_t bad;
	bip39_seed(ks->bip39, mnemonic);
	if (slip0010(ks->bip39, 64, sign_path, 3, ks->sign_seed, chain, &bad)) die("internal: slip0010 sign path");
	if (slip0010(ks->bip39, 64, ecdh_path, 3, ks->ecdh_scalar, chain, &bad)) die("internal: slip0010 ecdh path");
	uint8_t seedc[32];
	memcpy(seedc, ks->sign_seed, 32);
	crypto_ed25519_key_pair(ks->sk, ks->pk, seedc); /* wipes seedc */
	crypto_x25519_public_key(ks->xpk, ks->ecdh_scalar);
}

/* ------------------------------------------------------------------------ */
/* section 6.4 sealed packet                                                 */
/* ------------------------------------------------------------------------ */

#define SEAL_TAG_KEY "111/seal/v1"
#define SEAL_TAG_NONCE "111/seal-n/v1"
#define HDR_FIXED 11
#define MIN_PACKET (HDR_FIXED + 32 + 16)

static void seal_derive(uint8_t key[32], uint8_t nonce[24], const uint8_t shared[32], const uint8_t epk[32],
                        const uint8_t rpk[32])
{
	crypto_blake2b_ctx c;
	crypto_blake2b_init(&c, 32);
	crypto_blake2b_update(&c, (const uint8_t *)SEAL_TAG_KEY, strlen(SEAL_TAG_KEY));
	crypto_blake2b_update(&c, shared, 32);
	crypto_blake2b_update(&c, epk, 32);
	crypto_blake2b_update(&c, rpk, 32);
	crypto_blake2b_final(&c, key);
	crypto_blake2b_init(&c, 24);
	crypto_blake2b_update(&c, (const uint8_t *)SEAL_TAG_NONCE, strlen(SEAL_TAG_NONCE));
	crypto_blake2b_update(&c, epk, 32);
	crypto_blake2b_update(&c, rpk, 32);
	crypto_blake2b_final(&c, nonce);
}

/* header: version || mode || counter(LE) || extra_len || extra */
static size_t hdr_build(uint8_t *h, uint8_t version, uint8_t mode, uint64_t counter, const uint8_t *extra, size_t en)
{
	h[0] = version;
	h[1] = mode;
	store_le64(h + 2, counter);
	h[10] = (uint8_t)en;
	if (en) memcpy(h + 11, extra, en);
	return HDR_FIXED + en;
}

/* Seal with a caller-supplied ephemeral secret (deterministic). Returns packet
 * length, or 0 with `tok` = the seal-side policy error token. */
static size_t seal_with_esk(uint8_t *pkt, char tok[64], const uint8_t esk[32], const uint8_t rpk[32],
                            uint8_t version, uint8_t mode, uint64_t counter, const uint8_t *extra, size_t en,
                            const uint8_t sender[32], const uint8_t *pt, size_t pn)
{
	if (mode != 1) { snprintf(tok, 64, "UnsupportedCryptoMode:%u", mode); return 0; }
	if (en > 255) { snprintf(tok, 64, "HeaderTooLarge:%zu", en); return 0; }
	uint8_t epk[32], shared[32], key[32], nonce[24];
	if (x25519_low_order(rpk)) { snprintf(tok, 64, "Ecdh:LowOrderPublicKey"); return 0; }
	crypto_x25519_public_key(epk, esk);
	ecdh_err e = ecdh(shared, esk, rpk);
	if (e == E_NONCONTRIB) { snprintf(tok, 64, "Ecdh:NonContributory"); return 0; }
	seal_derive(key, nonce, shared, epk, rpk);
	size_t hl = hdr_build(pkt, version, mode, counter, extra, en);
	memcpy(pkt + hl, epk, 32);
	uint8_t *inner = xalloc(32 + pn);
	memcpy(inner, sender, 32);
	if (pn) memcpy(inner + 32, pt, pn);
	uint8_t mac[16];
	crypto_aead_lock(pkt + hl + 32, mac, key, nonce, pkt, hl, inner, 32 + pn);
	memcpy(pkt + hl + 32 + 32 + pn, mac, 16);
	crypto_wipe(shared, 32);
	crypto_wipe(key, 32);
	snprintf(tok, 64, "ok");
	return hl + 32 + 32 + pn + 16;
}

typedef struct {
	uint8_t version, mode;
	uint64_t counter;
	uint8_t extra[255];
	size_t extra_len;
	uint8_t sender[32];
	uint8_t *pt;
	size_t pt_len;
} opened;

/* Mirrors id111 seal::open ordering exactly. tok = "ok" or "err:<variant>". */
static int seal_open(const uint8_t rsk[32], const uint8_t *pkt, size_t n, opened *o, char tok[64])
{
	if (n < MIN_PACKET) { snprintf(tok, 64, "err:truncated"); return 0; }
	uint8_t version = pkt[0], mode = pkt[1];
	if (mode != 1) { snprintf(tok, 64, "err:mode:%u", mode); return 0; }
	size_t en = pkt[10], hl = HDR_FIXED + en;
	if (n < hl + 32 + 16) { snprintf(tok, 64, "err:truncated"); return 0; }
	const uint8_t *epk = pkt + hl;
	size_t ctn = n - hl - 32 - 16;
	const uint8_t *ct = pkt + hl + 32, *mac = pkt + n - 16;
	uint8_t shared[32], rpk[32], key[32], nonce[24];
	ecdh_err e = ecdh(shared, rsk, epk);
	if (e == E_LOWORDER) { snprintf(tok, 64, "err:ecdh:LowOrderPublicKey"); return 0; }
	if (e == E_NONCONTRIB) { snprintf(tok, 64, "err:ecdh:NonContributory"); return 0; }
	crypto_x25519_public_key(rpk, rsk);
	seal_derive(key, nonce, shared, epk, rpk);
	uint8_t *inner = xalloc(ctn);
	int bad = crypto_aead_unlock(inner, mac, key, nonce, pkt, hl, ct, ctn);
	crypto_wipe(shared, 32);
	crypto_wipe(key, 32);
	if (bad) { snprintf(tok, 64, "err:auth"); return 0; }
	if (ctn < 32) { snprintf(tok, 64, "err:short-plaintext"); return 0; }
	if (o) {
		o->version = version;
		o->mode = mode;
		o->counter = load_le64(pkt + 2);
		o->extra_len = en;
		memcpy(o->extra, pkt + HDR_FIXED, en);
		memcpy(o->sender, inner, 32);
		o->pt = inner + 32;
		o->pt_len = ctn - 32;
	}
	snprintf(tok, 64, "ok");
	return 1;
}

/* ------------------------------------------------------------------------ */
/* section 6.2 announce                                                      */
/* ------------------------------------------------------------------------ */

#define ANNOUNCE_TAG "111/announce/v1"
#define ANN_BASE 137
#define ANN_MAX 169

typedef struct { uint8_t id[32], enc[32]; uint64_t ctr; } pin;
typedef struct { pin e[16]; int n; } pinstore;

/* Mirrors id111 announce::process_announce ordering. */
static void announce_process(pinstore *st, const uint8_t own[32], const uint8_t *w, size_t n, char tok[96])
{
	if (n < ANN_BASE || n > ANN_MAX) { snprintf(tok, 96, "err:length:%zu", n); return; }
	if (w[0] != 1) { snprintf(tok, 96, "err:version:%u", w[0]); return; }
	const uint8_t *id = w + 1, *enc = w + 33, *sig = w + n - 64;
	if (x25519_low_order(enc)) { snprintf(tok, 96, "err:loworder"); return; }
	uint64_t ctr = load_le64(w + 65);
	verdict v = strict_verify_tagged(ANNOUNCE_TAG, w, n - 64, sig, id);
	if (v != V_OK) { snprintf(tok, 96, "err:sig:%s", verdict_name(v)); return; }
	if (memcmp(id, own, 32) == 0) { snprintf(tok, 96, "err:collision"); return; }
	for (int i = 0; i < st->n; i++) {
		if (memcmp(st->e[i].id, id, 32) != 0) continue;
		if (memcmp(st->e[i].enc, enc, 32) != 0) { snprintf(tok, 96, "err:conflict"); return; }
		if (ctr <= st->e[i].ctr) {
			snprintf(tok, 96, "err:stale:%llu:%llu", (unsigned long long)st->e[i].ctr, (unsigned long long)ctr);
			return;
		}
		unsigned long long prev = st->e[i].ctr;
		st->e[i].ctr = ctr;
		snprintf(tok, 96, "ok:updated:%llu", prev);
		return;
	}
	if (st->n >= 16) die("internal: harness pin store full");
	memcpy(st->e[st->n].id, id, 32);
	memcpy(st->e[st->n].enc, enc, 32);
	st->e[st->n].ctr = ctr;
	st->n++;
	snprintf(tok, 96, "ok:pinned");
}

/* ------------------------------------------------------------------------ */
/* section 10 attestation (node leg only; transport_sig is opaque / Layer 7) */
/* ------------------------------------------------------------------------ */

#define ATT_NODE_TAG "111/attest-node/v1"
#define ATT_TRANSPORT_TAG "111/attest-transport/v1"
#define ATT_LEN 202

/* signed input: tag || version || node_pubkey || transport_key || transport_id || expiry(8 LE)
 * (NOTE transport_key BEFORE transport_id: differs from the wire order). */
static size_t att_node_input(uint8_t out[1 + 32 + 32 + 1 + 8], const uint8_t *w)
{
	out[0] = w[0];
	memcpy(out + 1, w + 1, 32);   /* node_pubkey */
	memcpy(out + 33, w + 34, 32); /* transport_key */
	out[65] = w[33];              /* transport_id */
	memcpy(out + 66, w + 66, 8);  /* expiry_epoch */
	return 74;
}

static void att_node_leg(const uint8_t *w, size_t n, char tok[96])
{
	if (n != ATT_LEN) { snprintf(tok, 96, "err:length:%zu", n); return; }
	if (w[0] != 1) { snprintf(tok, 96, "err:version:%u", w[0]); return; }
	if (w[33] != 1) { snprintf(tok, 96, "err:tid:%u", w[33]); return; }
	uint8_t in[74];
	size_t inn = att_node_input(in, w);
	verdict v = strict_verify_tagged(ATT_NODE_TAG, in, inn, w + 74, w + 1);
	if (v != V_OK) snprintf(tok, 96, "err:node-sig:%s", verdict_name(v));
	else snprintf(tok, 96, "ok");
}

/* ------------------------------------------------------------------------ */
/* per-file handlers                                                         */
/* ------------------------------------------------------------------------ */

/* delta report records (filled by the edge handler, printed at the end) */
typedef struct {
	char name[112];
	int oracle_ok, raw_ok, wrapped_ok;
	verdict v;
} edge_row;
static edge_row g_edge[64];
static int g_nedge;
static int g_x_low_total, g_x_low_raw_zero, g_x_low_raw_nonzero;

static void h_selftest(void)
{
	/* Vendored-library sanity (well-known public KATs; build/miscompile guard). */
	uint8_t h[64];
	crypto_blake2b(h, 64, (const uint8_t *)"abc", 3);
	expect_hex("BLAKE2b-512(abc) RFC 7693", "digest", h, 64,
	           "ba80a53f981c4d0d6a2797b69f12f6e94c212f14685ac4b74b12bb6fdbffa2d17d87c5392aab792dc252d5de4533cc9518d38aa8dbf1925ab92386edd4009923");
	crypto_blake2b(h, 24, (const uint8_t *)"abc", 3);
	expect_hex("BLAKE2b-192(abc)", "digest (outlen is a parameter, not a truncation)", h, 24,
	           "56a17e38cc371a46b12c32f18e0c61de2a84e9c2555b114e");
	crypto_sha512(h, (const uint8_t *)"abc", 3);
	expect_hex("SHA-512(abc) FIPS 180", "digest", h, 64,
	           "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f");
	uint8_t key[20];
	memset(key, 0x0b, sizeof key);
	crypto_sha512_hmac(h, key, sizeof key, (const uint8_t *)"Hi There", 8);
	expect_hex("HMAC-SHA512 RFC 4231 case 1", "mac", h, 64,
	           "87aa7cdea5ef619d4ff0b4241a1d6cb02379f4e2ce4ec2787ad0b30545e17cdedaa833b7d6b8a702038b274eaea3f4e4be9d914eeb61f1702e696c203a126854");
	/* mnemonic -> BIP-39 seed (PBKDF2 over the long-key HMAC path is exercised by the 24-word vector) */
	uint8_t seed[64];
	bip39_seed(seed, "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about");
	expect_hex("BIP-39 seed (canonical-12)", "PBKDF2-HMAC-SHA512 x2048", seed, 64,
	           "5eb00bbddcf069084889a8ab9155568165f5c453ccb85e70811aaed6f6da5fc19a5ac40b389cd370d086206dec8aa6c43daea6690f20ad3d8d48b2d2ce9e38e4");
	/* the decode probe must separate on-curve from off-curve */
	uint8_t y2[32] = { 2 }, y3[32] = { 3 };
	CHECK(!point_decodes(y2), "point_decodes: y=2 is not on the curve");
	CHECK(point_decodes(y3), "point_decodes: y=3 decodes");
	/* the v2.1 [l]P reference arithmetic agrees with Monocypher's (8th KAT) */
	uint8_t one[32] = { 1 }, benc[32], own[32];
	gf B[4], Q[4];
	crypto_eddsa_scalarbase(benc, one);
	CHECK(ge_decode(B, benc) == 0, "[l]P reference: decodes Monocypher's base point");
	ge_encode(own, B);
	CHECK(memcmp(own, benc, 32) == 0, "[l]P reference: base point decode/encode round-trip");
	CHECK(point_torsion_free(benc), "[l]P reference: [l]B = O");
	for (int k = 0; k < 6; k++) {
		/* arbitrary 256-bit scalars (not reduced, top bits set for k >= 3), and l - 1 */
		uint8_t sc[32], mono[32];
		crypto_blake2b(sc, 32, (const uint8_t *)&k, sizeof k);
		if (k >= 3) sc[31] |= 0xf0;
		if (k == 5) {
			memcpy(sc, GROUP_L, 32);
			sc[0]--; /* l - 1: [l-1]B = -B */
		}
		crypto_eddsa_scalarbase(mono, sc);
		ge_decode(B, benc);
		ge_scalarmult(Q, B, sc);
		ge_encode(own, Q);
		CHECK(memcmp(own, mono, 32) == 0, "[l]P reference: [s]B == crypto_eddsa_scalarbase(s), scalar %d", k);
	}
	/* a mixed-order point B + T8 is not torsion-free; its [8] multiple is */
	uint8_t t8[32], mixed[32], cleared[32];
	gf T[4];
	hex32(t8, "c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac037a");
	CHECK(ge_decode(T, t8) == 0 && ge_decode(B, benc) == 0, "[l]P reference: decodes T8");
	ge_add(B, T);
	ge_encode(mixed, B);
	CHECK(!point_torsion_free(mixed) && !point_small_order(mixed), "[l]P reference: B + T8 is mixed-order");
	uint8_t eight[32] = { 8 };
	ge_decode(B, mixed);
	ge_scalarmult(Q, B, eight);
	ge_encode(cleared, Q);
	CHECK(point_torsion_free(cleared), "[l]P reference: [8](B + T8) is torsion-free");
	cur->vectors = cur->expected_vectors = 8;
}

static void h_ed25519(void)
{
	const jv *root = load_json("ed25519-rfc8032.json");
	const jv *vs = jarr(root, "vectors");
	cur->expected_vectors = (int)vs->n;
	for (size_t i = 0; i < vs->n; i++) {
		const jv *v = vs->kid[i];
		cur->vectors++;
		const char *name = jstr(v, "name");
		uint8_t seed[32], pk[32], sig[64], *msg;
		jhexn(v, "secret_key", seed, 32);
		jhexn(v, "public_key", pk, 32);
		jhexn(v, "signature", sig, 64);
		size_t mn = (size_t)jhex(v, "message", &msg);

		uint8_t sk[64], pk2[32], seedc[32], sig2[64];
		memcpy(seedc, seed, 32);
		crypto_ed25519_key_pair(sk, pk2, seedc);
		CHECK(memcmp(pk2, pk, 32) == 0, "%s: keygen-from-seed pubkey == frozen", name);
		CHECK(memcmp(sk, seed, 32) == 0 && memcmp(sk + 32, pk, 32) == 0, "%s: secret_key = seed||pubkey", name);
		crypto_ed25519_sign(sig2, sk, msg, mn);
		CHECK(memcmp(sig2, sig, 64) == 0, "%s: deterministic sign == frozen signature", name);
		CHECK(crypto_ed25519_check(sig, pk, msg, mn) == 0, "%s: raw crypto_ed25519_check accepts", name);
		CHECK(strict_verify(sig, pk, msg, mn) == V_OK, "%s: strict_verify accepts", name);
		/* tamper controls (not frozen data): each must be rejected */
		uint8_t *m2 = xalloc(mn + 1);
		memcpy(m2, msg, mn);
		if (mn) m2[0] ^= 1;
		else m2[0] = 0x42;
		CHECK(strict_verify(sig, pk, m2, mn ? mn : 1) != V_OK, "%s: modified message rejected", name);
		uint8_t s2[64];
		memcpy(s2, sig, 64);
		s2[5] ^= 0x10;
		CHECK(strict_verify(s2, pk, msg, mn) != V_OK, "%s: flipped R bit rejected", name);
		memcpy(s2, sig, 64);
		s2[40] ^= 0x01;
		CHECK(strict_verify(s2, pk, msg, mn) != V_OK, "%s: flipped S bit rejected", name);
	}
}

static void h_x25519(void)
{
	const jv *root = load_json("x25519-rfc7748.json");
	const jv *dh = jfield(root, "diffie_hellman");
	uint8_t as[32], ap[32], bs[32], bp[32], sh[32], t[32], t2[32];
	jhexn(dh, "alice_scalar", as, 32);
	jhexn(dh, "alice_public", ap, 32);
	jhexn(dh, "bob_scalar", bs, 32);
	jhexn(dh, "bob_public", bp, 32);
	jhexn(dh, "shared", sh, 32);
	cur->vectors++;
	crypto_x25519_public_key(t, as);
	CHECK(memcmp(t, ap, 32) == 0, "RFC 7748 6.1: alice public from scalar");
	crypto_x25519_public_key(t, bs);
	CHECK(memcmp(t, bp, 32) == 0, "RFC 7748 6.1: bob public from scalar");
	CHECK(ecdh(t, as, bp) == E_OK && memcmp(t, sh, 32) == 0, "RFC 7748 6.1: X25519(alice, bob_pub) == shared");
	CHECK(ecdh(t2, bs, ap) == E_OK && memcmp(t2, sh, 32) == 0, "RFC 7748 6.1: X25519(bob, alice_pub) == shared");

	const jv *sm = jarr(root, "scalarmult");
	for (size_t i = 0; i < sm->n; i++) {
		cur->vectors++;
		uint8_t s[32], u[32], out[32];
		jhexn(sm->kid[i], "scalar", s, 32);
		jhexn(sm->kid[i], "u", u, 32);
		jhexn(sm->kid[i], "output", out, 32);
		crypto_x25519(t, s, u);
		CHECK(memcmp(t, out, 32) == 0, "RFC 7748 5.2 #%zu: raw crypto_x25519", i + 1);
		CHECK(ecdh(t2, s, u) == E_OK && memcmp(t2, out, 32) == 0, "RFC 7748 5.2 #%zu: wrapped ecdh", i + 1);
	}

	const jv *it = jfield(root, "iterated");
	cur->vectors++;
	uint8_t k[32], u[32];
	jhexn(it, "start", k, 32);
	memcpy(u, k, 32);
	for (int n = 1; n <= 1000; n++) {
		crypto_x25519(t, k, u);
		memcpy(u, k, 32);
		memcpy(k, t, 32);
		if (n == 1) expect_hex("RFC 7748 5.2 iterated", "after 1 iteration", k, 32, jstr(it, "after_1"));
	}
	expect_hex("RFC 7748 5.2 iterated", "after 1000 iterations", k, 32, jstr(it, "after_1000"));
	/* companion cross-link between the two RFC files */
	CHECK(strcmp(jstr(root, "ed25519_companion_seed"), "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60") == 0,
	      "ed25519_companion_seed is RFC 8032 TEST 1");
	cur->expected_vectors = 1 + (int)sm->n + 1;
}

static void h_slip0010_published(void)
{
	const jv *root = load_json("slip0010-published.json");
	const jv *vs = jarr(root, "vectors");
	int expected = 0;
	for (size_t i = 0; i < vs->n; i++) {
		const jv *v = vs->kid[i];
		const jv *chains = jarr(v, "chains");
		expected += (int)chains->n;
		uint8_t *seed;
		long sn = jhex(v, "seed", &seed);
		for (size_t c = 0; c < chains->n; c++) {
			const jv *ch = chains->kid[c];
			cur->vectors++;
			const char *path = jstr(ch, "path");
			const jv *ix = jarr(ch, "indices");
			uint32_t idx[16];
			if (ix->n > 16) die("slip0010-published: path too long");
			for (size_t j = 0; j < ix->n; j++) {
				if (ix->kid[j]->t != J_NUM) die("slip0010-published: non-numeric index");
				idx[j] = (uint32_t)ix->kid[j]->i;
			}
			uint8_t key[32], chain[32];
			uint32_t bad;
			int rc = slip0010(seed, (size_t)sn, idx, ix->n, key, chain, &bad);
			CHECK(rc == 0, "%s %s: derivation ok", jstr(v, "name"), path);
			expect_hex(path, "key half", key, 32, jstr(ch, "key"));
			expect_hex(path, "chain half", chain, 32, jstr(ch, "chain"));
		}
	}
	cur->expected_vectors = expected;
}

static void h_slip0010_project(void)
{
	const jv *root = load_json("slip0010-project-paths.json");
	const jv *vs = jarr(root, "vectors");
	const jv *neg = jarr(root, "negatives");
	cur->expected_vectors = (int)(vs->n + neg->n);
	for (size_t i = 0; i < vs->n; i++) {
		const jv *v = vs->kid[i];
		cur->vectors++;
		const char *name = jstr(v, "name");
		keyset ks;
		keyset_from_mnemonic(&ks, jstr(v, "mnemonic"));
		expect_hex(name, "BIP-39 seed (PBKDF2-HMAC-SHA512, empty passphrase)", ks.bip39, 64, jstr(v, "bip39_seed"));
		expect_hex(name, "m/111'/0'/0' signing seed", ks.sign_seed, 32, jstr(v, "signing_seed"));
		expect_hex(name, "m/111'/0'/1' X25519 scalar", ks.ecdh_scalar, 32, jstr(v, "ecdh_scalar"));
	}
	for (size_t i = 0; i < neg->n; i++) {
		const jv *v = neg->kid[i];
		cur->vectors++;
		const jv *ix = jarr(v, "indices");
		uint32_t idx[16];
		for (size_t j = 0; j < ix->n && j < 16; j++) idx[j] = (uint32_t)ix->kid[j]->i;
		uint8_t key[32], chain[32], seed[64] = { 0 };
		uint32_t bad = 0;
		char tok[64];
		int rc = slip0010(seed, 64, idx, ix->n, key, chain, &bad);
		if (rc) snprintf(tok, sizeof tok, "IndexOutOfRange:%u", bad);
		else snprintf(tok, sizeof tok, "ok");
		CHECK(strcmp(tok, jstr(v, "expect")) == 0, "%s: got %s want %s", jstr(v, "name"), tok, jstr(v, "expect"));
	}
}

static void h_keyset_project(void)
{
	const jv *root = load_json("keyset-project.json");
	const jv *vs = jarr(root, "vectors");
	cur->expected_vectors = (int)vs->n;
	for (size_t i = 0; i < vs->n; i++) {
		const jv *v = vs->kid[i];
		cur->vectors++;
		const char *name = jstr(v, "name");
		keyset ks;
		keyset_from_mnemonic(&ks, jstr(v, "mnemonic"));
		expect_hex(name, "NodeId (Ed25519 pubkey from SLIP-0010 seed)", ks.pk, 32, jstr(v, "node_id"));
		expect_hex(name, "X25519 public key", ks.xpk, 32, jstr(v, "x25519_public"));
		uint8_t pub[64], priv[64];
		memcpy(pub, ks.pk, 32);
		memcpy(pub + 32, ks.xpk, 32);
		memcpy(priv, ks.sign_seed, 32);
		memcpy(priv + 32, ks.ecdh_scalar, 32);
		expect_hex(name, "64B public keyset", pub, 64, jstr(v, "public_keyset"));
		expect_hex(name, "64B private keyset (seed || scalar)", priv, 64, jstr(v, "private_keyset"));
		/* the derived keys must work end to end: sign + strict-verify, ECDH self-consistency */
		uint8_t sig[64];
		crypto_ed25519_sign(sig, ks.sk, (const uint8_t *)"111", 3);
		CHECK(strict_verify(sig, ks.pk, (const uint8_t *)"111", 3) == V_OK, "%s: sign/strict-verify under derived key", name);
		uint8_t es[32], ep[32], s1[32], s2[32];
		memset(es, 0x42, 32);
		crypto_x25519_public_key(ep, es);
		CHECK(ecdh(s1, es, ks.xpk) == E_OK && ecdh(s2, ks.ecdh_scalar, ep) == E_OK && memcmp(s1, s2, 32) == 0,
		      "%s: ECDH agrees both directions", name);
	}
}

static void h_edge(void)
{
	const jv *root = load_json("edge-vectors.json");
	const jv *k = jfield(root, "constants");
	const jv *tors = jarr(root, "torsion_subgroup");
	const jv *ed = jarr(root, "ed25519_strict");
	const jv *xl = jarr(root, "x25519_low_order");
	const jv *xc = jarr(root, "x25519_accepted_controls");
	cur->expected_vectors = (int)(1 + tors->n + ed->n + xl->n + xc->n);

	/* constants the wrapper hard-codes must equal the oracle's */
	cur->vectors++;
	uint8_t b[32];
	jhexn(k, "group_order_l_le", b, 32);
	CHECK(memcmp(b, GROUP_L, 32) == 0, "wrapper l == oracle group_order_l_le");
	jhexn(k, "field_prime_p_le", b, 32);
	CHECK(memcmp(b, FIELD_P, 32) == 0, "wrapper p == oracle field_prime_p_le");
	jhexn(k, "dual_encoding_canonical_y", b, 32);
	CHECK(point_canonical(b), "dual-encoding canonical y accepted as canonical");
	jhexn(k, "dual_encoding_noncanonical_y_plus_p", b, 32);
	CHECK(!point_canonical(b), "dual-encoding y+p rejected as non-canonical");

	/* the 8-element torsion subgroup: every encoding must be classified small-order */
	for (size_t i = 0; i < tors->n; i++) {
		cur->vectors++;
		uint8_t e[32];
		jhexn(tors->kid[i], "encoding", e, 32);
		CHECK(point_canonical(e) && point_small_order(e) && point_decodes(e), "torsion %s: canonical, decodes, small-order",
		      jstr(tors->kid[i], "name"));
		CHECK(point_torsion_free(e) == (strcmp(jstr(tors->kid[i], "order"), "1") == 0),
		      "torsion %s: [l]T = O iff T is the identity (v2.1 leg)", jstr(tors->kid[i], "name"));
	}

	/* Ed25519 strict predicate: oracle vs wrapped vs raw */
	int oracle_rej = 0, raw_acc_of_rej = 0, wrapped_acc_of_rej = 0, exact_variant = 0;
	int leg[6] = { 0 };
	for (size_t i = 0; i < ed->n; i++) {
		const jv *v = ed->kid[i];
		cur->vectors++;
		const char *name = jstr(v, "name"), *expect = jstr(v, "expect");
		uint8_t pk[32], sig[64], *msg;
		jhexn(v, "public_key", pk, 32);
		jhexn(v, "signature", sig, 64);
		size_t mn = (size_t)jhex(v, "message", &msg);
		verdict vd = strict_verify(sig, pk, msg, mn);
		int raw = crypto_ed25519_check(sig, pk, msg, mn) == 0;
		int oracle_ok = strcmp(expect, "ok") == 0;
		int is_err = strncmp(expect, "err:", 4) == 0;
		if (!oracle_ok && !is_err) {
			check_(0, "%s: unknown expect token \"%s\"", name, expect);
			continue;
		}
		char want[64];
		snprintf(want, sizeof want, "%s", oracle_ok ? "ok" : expect + 4);
		CHECK((vd == V_OK) == oracle_ok, "%s: strict accept/reject agrees with oracle (oracle %s, C %s)", name, expect,
		      verdict_name(vd));
		int exact = strcmp(verdict_name(vd), want) == 0;
		CHECK(exact, "%s: exact VerifyError variant (want %s, got %s)", name, want, verdict_name(vd));
		exact_variant += exact;
		if (!oracle_ok) {
			oracle_rej++;
			leg[vd]++;
			raw_acc_of_rej += raw;
			wrapped_acc_of_rej += (vd == V_OK);
		}
		if (g_nedge < 64) {
			edge_row *r = &g_edge[g_nedge++];
			snprintf(r->name, sizeof r->name, "%s", name);
			r->oracle_ok = oracle_ok;
			r->raw_ok = raw;
			r->wrapped_ok = (vd == V_OK);
			r->v = vd;
		}
	}
	printf("  [edge-vectors.json] ed25519_strict: %zu vectors, %d oracle-rejects; raw crypto_ed25519_check ACCEPTED %d of them; "
	       "wrapped accepted %d; exact variant match %d/%zu\n",
	       ed->n, oracle_rej, raw_acc_of_rej, wrapped_acc_of_rej, exact_variant, ed->n);
	printf("  [edge-vectors.json] rejecting leg histogram: NonCanonicalPublicKey=%d NonCanonicalR=%d NonCanonicalS=%d "
	       "MalformedPublicKey=%d InvalidSignature(small-order/torsion/undecodable R/equation)=%d\n",
	       leg[V_NONCANON_A], leg[V_NONCANON_R], leg[V_NONCANON_S], leg[V_MALFORMED_A], leg[V_INVALID]);

	/* X25519 low-order class */
	uint8_t rsk[32], out[32], dummy[32];
	memset(rsk, 0x11, 32);
	for (size_t i = 0; i < xl->n; i++) {
		const jv *v = xl->kid[i];
		cur->vectors++;
		uint8_t u[32];
		jhexn(v, "u", u, 32);
		const char *name = jstr(v, "name");
		CHECK(strcmp(jstr(v, "expect"), "LowOrderPublicKey") == 0, "%s: oracle token is LowOrderPublicKey", name);
		CHECK(x25519_low_order(u), "%s: ingress blacklist rejects", name);
		CHECK(ecdh(dummy, rsk, u) == E_LOWORDER, "%s: ecdh() refuses (LowOrderPublicKey)", name);
		crypto_x25519(out, rsk, u);
		int z = all_zero(out, 32);
		g_x_low_total++;
		if (z) g_x_low_raw_zero++;
		else g_x_low_raw_nonzero++;
	}
	for (size_t i = 0; i < xc->n; i++) {
		const jv *v = xc->kid[i];
		cur->vectors++;
		uint8_t u[32];
		jhexn(v, "u", u, 32);
		const char *name = jstr(v, "name");
		CHECK(!x25519_low_order(u), "%s: control passes ingress", name);
		CHECK(ecdh(out, rsk, u) == E_OK && !all_zero(out, 32), "%s: control ECDH contributory", name);
	}
}

static void h_announce(void)
{
	const jv *root = load_json("announce.json");
	const jv *signers = jfield(root, "signers");
	const jv *kat = jfield(root, "kat");
	const jv *pos = jarr(root, "positives");
	const jv *mal = jarr(root, "malformed");
	const jv *scn = jarr(root, "scenarios");
	int expected = 1 + (int)pos->n + (int)mal->n;
	for (size_t i = 0; i < scn->n; i++) expected += (int)jarr(scn->kid[i], "steps")->n;
	cur->expected_vectors = expected;

	CHECK(strcmp(jstr(root, "tag"), ANNOUNCE_TAG) == 0, "domain tag is %s", ANNOUNCE_TAG);
	keyset ka, kb;
	keyset_from_mnemonic(&ka, jstr(jfield(signers, "a"), "mnemonic"));
	keyset_from_mnemonic(&kb, jstr(jfield(signers, "b"), "mnemonic"));
	expect_hex("signer a", "node_id", ka.pk, 32, jstr(jfield(signers, "a"), "node_id"));
	expect_hex("signer a", "enc_pubkey", ka.xpk, 32, jstr(jfield(signers, "a"), "enc_pubkey"));
	expect_hex("signer b", "node_id", kb.pk, 32, jstr(jfield(signers, "b"), "node_id"));
	expect_hex("signer b", "enc_pubkey", kb.xpk, 32, jstr(jfield(signers, "b"), "enc_pubkey"));

	/* KAT: rebuild the head, re-sign deterministically, compare to the frozen bytes */
	cur->vectors++;
	uint8_t *appdata, *wire, *si, sig[64];
	size_t an = (size_t)jhex(kat, "appdata", &appdata);
	size_t wn = (size_t)jhex(kat, "wire", &wire);
	size_t sin = (size_t)jhex(kat, "signed_input", &si);
	jhexn(kat, "signature", sig, 64);
	uint64_t ctr = jdec64(kat, "counter");
	CHECK(strcmp(jstr(kat, "signer"), "a") == 0, "kat signer is a");
	uint8_t *head = xalloc(73 + an);
	head[0] = 1;
	memcpy(head + 1, ka.pk, 32);
	memcpy(head + 33, ka.xpk, 32);
	store_le64(head + 65, ctr);
	if (an) memcpy(head + 73, appdata, an);
	uint8_t *want_si = xalloc(strlen(ANNOUNCE_TAG) + 73 + an);
	memcpy(want_si, ANNOUNCE_TAG, strlen(ANNOUNCE_TAG));
	memcpy(want_si + strlen(ANNOUNCE_TAG), head, 73 + an);
	CHECK(sin == strlen(ANNOUNCE_TAG) + 73 + an && memcmp(si, want_si, sin) == 0, "kat: signed_input == tag || head");
	uint8_t sig2[64];
	sign_tagged(sig2, ka.sk, ANNOUNCE_TAG, head, 73 + an);
	CHECK(memcmp(sig2, sig, 64) == 0, "kat: C re-sign == frozen signature");
	CHECK(wn == 73 + an + 64 && memcmp(wire, head, 73 + an) == 0 && memcmp(wire + 73 + an, sig, 64) == 0,
	      "kat: wire == head || signature");
	CHECK(strict_verify_tagged(ANNOUNCE_TAG, head, 73 + an, sig, ka.pk) == V_OK, "kat: strict verify under the announce tag");
	CHECK(strict_verify(sig, ka.pk, head, 73 + an) != V_OK, "kat: UNTAGGED verify of the same signature is rejected (domain separation)");

	/* positives: all verify, and all re-derive byte-exactly (they are signer-a artifacts) */
	for (size_t i = 0; i < pos->n; i++) {
		const jv *v = pos->kid[i];
		cur->vectors++;
		const char *name = jstr(v, "name");
		uint8_t *w, *ad;
		size_t n = (size_t)jhex(v, "wire", &w);
		size_t adn = (size_t)jhex(v, "appdata", &ad);
		pinstore st;
		memset(&st, 0, sizeof st);
		char tok[96];
		announce_process(&st, kb.pk, w, n, tok);
		CHECK(strcmp(tok, "ok:pinned") == 0, "%s: processes to ok:pinned (got %s)", name, tok);
		uint8_t *h2 = xalloc(73 + adn);
		h2[0] = 1;
		memcpy(h2 + 1, ka.pk, 32);
		memcpy(h2 + 33, ka.xpk, 32);
		store_le64(h2 + 65, jdec64(v, "counter"));
		if (adn) memcpy(h2 + 73, ad, adn);
		uint8_t s3[64];
		sign_tagged(s3, ka.sk, ANNOUNCE_TAG, h2, 73 + adn);
		CHECK(n == 73 + adn + 64 && memcmp(w, h2, 73 + adn) == 0 && memcmp(w + 73 + adn, s3, 64) == 0,
		      "%s: C-built wire == frozen wire", name);
	}

	/* malformed: each alone against a fresh store, own = canonical-24 (signer b) */
	for (size_t i = 0; i < mal->n; i++) {
		const jv *v = mal->kid[i];
		cur->vectors++;
		const char *name = jstr(v, "name"), *expect = jstr(v, "expect");
		uint8_t *w;
		size_t n = (size_t)jhex(v, "wire", &w);
		pinstore st;
		memset(&st, 0, sizeof st);
		char tok[96];
		announce_process(&st, kb.pk, w, n, tok);
		CHECK(strcmp(tok, expect) == 0, "%s: got %s want %s", name, tok, expect);
		CHECK(st.n == 0, "%s: store left empty", name);
	}

	/* scenarios: ordered steps against one store */
	for (size_t i = 0; i < scn->n; i++) {
		const jv *s = scn->kid[i];
		uint8_t own[32];
		jhexn(s, "own_node_id", own, 32);
		const jv *steps = jarr(s, "steps");
		pinstore st;
		memset(&st, 0, sizeof st);
		for (size_t j = 0; j < steps->n; j++) {
			cur->vectors++;
			uint8_t *w;
			size_t n = (size_t)jhex(steps->kid[j], "wire", &w);
			char tok[96];
			announce_process(&st, own, w, n, tok);
			const char *expect = jstr(steps->kid[j], "expect");
			CHECK(strcmp(tok, expect) == 0, "scenario \"%s\" step %zu: got %s want %s", jstr(s, "name"), j + 1, tok, expect);
		}
	}
}

static void h_sealed(void)
{
	const jv *root = load_json("sealed-packet.json");
	const jv *rcp = jfield(root, "recipient");
	const jv *kat = jfield(root, "kat");
	const jv *wr = jfield(root, "wrong_recipient");
	const jv *neg = jarr(root, "negatives");
	const jv *sneg = jarr(root, "seal_negatives");
	cur->expected_vectors = (int)(1 + 1 + neg->n + sneg->n);

	uint8_t rsk[32], rpk[32], t[32];
	jhexn(rcp, "x25519_scalar", rsk, 32);
	jhexn(rcp, "x25519_public", rpk, 32);
	crypto_x25519_public_key(t, rsk);
	CHECK(memcmp(t, rpk, 32) == 0, "recipient X25519 public from scalar");

	/* ---- KAT: seal (deterministic esk) and open ---- */
	cur->vectors++;
	uint8_t esk[32], epk[32], rpk2[32], shared[32], sender[32], key[32], nonce[24];
	uint8_t *extra, *pt, *inner, *hb, *ctt, *packet;
	jhexn(kat, "esk", esk, 32);
	jhexn(kat, "epk", epk, 32);
	jhexn(kat, "rpk", rpk2, 32);
	jhexn(kat, "shared", shared, 32);
	jhexn(kat, "sender_node_id", sender, 32);
	size_t en = (size_t)jhex(jfield(kat, "header"), "extra", &extra);
	size_t pn = (size_t)jhex(kat, "plaintext", &pt);
	size_t inn = (size_t)jhex(kat, "inner_plaintext", &inner);
	size_t hbn = (size_t)jhex(kat, "header_bytes", &hb);
	size_t ctn = (size_t)jhex(kat, "ciphertext_and_tag", &ctt);
	size_t pkn = (size_t)jhex(kat, "packet", &packet);
	const jv *hdr = jfield(kat, "header");
	uint8_t version = (uint8_t)jint(hdr, "version"), mode = (uint8_t)jint(hdr, "crypto_mode");
	uint64_t counter = jdec64(hdr, "counter");

	CHECK(memcmp(rpk2, rpk, 32) == 0, "kat: rpk is the recipient public key");
	crypto_x25519_public_key(t, esk);
	CHECK(memcmp(t, epk, 32) == 0, "kat: epk = X25519_base(esk)");
	uint8_t sh2[32];
	CHECK(ecdh(sh2, esk, rpk) == E_OK && memcmp(sh2, shared, 32) == 0, "kat: shared = X25519(esk, rpk)");
	seal_derive(key, nonce, sh2, epk, rpk);
	expect_hex("kat", "key = BLAKE2b-256('111/seal/v1' || shared || epk || rpk)", key, 32, jstr(kat, "key"));
	expect_hex("kat", "nonce = BLAKE2b-192('111/seal-n/v1' || epk || rpk)", nonce, 24, jstr(kat, "nonce"));
	uint8_t hdrb[HDR_FIXED + 255];
	size_t hl = hdr_build(hdrb, version, mode, counter, extra, en);
	CHECK(hl == hbn && memcmp(hdrb, hb, hbn) == 0, "kat: header bytes == frozen header_bytes");
	CHECK(inn == 32 + pn && memcmp(inner, sender, 32) == 0 && memcmp(inner + 32, pt, pn) == 0,
	      "kat: inner plaintext = sender_node_id || plaintext");
	uint8_t *outp = xalloc(pkn + 64);
	char tok[64];
	size_t sn = seal_with_esk(outp, tok, esk, rpk, version, mode, counter, extra, en, sender, pt, pn);
	CHECK(sn == pkn && memcmp(outp, packet, pkn) == 0, "kat: C seal(esk, rpk, header, plaintext) == frozen packet (%s)", tok);
	CHECK(pkn == hbn + 32 + ctn && memcmp(packet + hbn + 32, ctt, ctn) == 0, "kat: packet tail == ciphertext_and_tag");
	opened o;
	int ok = seal_open(rsk, packet, pkn, &o, tok);
	CHECK(ok, "kat: C opens the frozen packet (%s)", tok);
	if (ok) {
		CHECK(memcmp(o.sender, sender, 32) == 0, "kat: opened sender == sender_node_id");
		CHECK(o.pt_len == pn && memcmp(o.pt, pt, pn) == 0, "kat: opened plaintext == frozen plaintext");
		CHECK(o.counter == counter && o.version == version && o.mode == mode && o.extra_len == en &&
		      memcmp(o.extra, extra, en) == 0, "kat: opened header fields");
	}

	/* ---- wrong recipient: canonical-12 keyset ---- */
	cur->vectors++;
	keyset k12;
	keyset_from_mnemonic(&k12, jstr(wr, "mnemonic"));
	ok = seal_open(k12.ecdh_scalar, packet, pkn, NULL, tok);
	CHECK(!ok && strcmp(tok, jstr(wr, "expect")) == 0, "wrong recipient: got %s want %s", tok, jstr(wr, "expect"));

	/* ---- negatives ---- */
	for (size_t i = 0; i < neg->n; i++) {
		const jv *v = neg->kid[i];
		cur->vectors++;
		uint8_t *p;
		size_t n = (size_t)jhex(v, "packet", &p);
		ok = seal_open(rsk, p, n, NULL, tok);
		CHECK(!ok && strcmp(tok, jstr(v, "expect")) == 0, "%s: got %s want %s", jstr(v, "name"), tok, jstr(v, "expect"));
	}

	/* ---- seal-side policy negatives ---- */
	for (size_t i = 0; i < sneg->n; i++) {
		const jv *v = sneg->kid[i];
		cur->vectors++;
		uint8_t rcv[32];
		jhexn(v, "recipient", rcv, 32);
		uint8_t m = (uint8_t)jint(v, "crypto_mode");
		long long el = jint(v, "extra_len");
		uint8_t zeros[256] = { 0 };
		uint8_t *buf = xalloc(HDR_FIXED + 512 + 100);
		size_t r = seal_with_esk(buf, tok, esk, rcv, 1, m, 1, zeros, (size_t)el, sender, pt, pn);
		CHECK(r == 0 && strcmp(tok, jstr(v, "expect")) == 0, "%s: got %s want %s", jstr(v, "name"), tok, jstr(v, "expect"));
	}

	/* ---- C-side round-trip sweep (not frozen data): lengths straddling the
	 *      ChaCha20 64B block / Poly1305 16B block boundaries and every header size ---- */
	static const size_t plens[] = { 0, 1, 15, 16, 17, 31, 32, 33, 63, 64, 65, 127, 128, 129, 200 };
	static const size_t elens[] = { 0, 1, 2, 63, 255 };
	int rt_ok = 1, rt_n = 0;
	for (size_t a = 0; a < sizeof plens / sizeof *plens; a++) {
		for (size_t b = 0; b < sizeof elens / sizeof *elens; b++) {
			uint8_t e2[32], ex[255], msg[200];
			crypto_blake2b(e2, 32, (const uint8_t *)&plens[a], sizeof plens[a]);
			e2[1] ^= (uint8_t)b;
			for (size_t q = 0; q < sizeof ex; q++) ex[q] = (uint8_t)(q * 7 + b);
			for (size_t q = 0; q < sizeof msg; q++) msg[q] = (uint8_t)(q * 13 + a);
			uint8_t *pk2 = xalloc(HDR_FIXED + 255 + 32 + 32 + 200 + 16);
			size_t l = seal_with_esk(pk2, tok, e2, rpk, 1, 1, (uint64_t)(a * 100 + b), ex, elens[b], sender, msg, plens[a]);
			opened oo;
			memset(&oo, 0, sizeof oo);
			int good = l == HDR_FIXED + elens[b] + 32 + 32 + plens[a] + 16 && seal_open(rsk, pk2, l, &oo, tok) &&
			           oo.pt_len == plens[a] && (plens[a] == 0 || memcmp(oo.pt, msg, plens[a]) == 0) &&
			           memcmp(oo.sender, sender, 32) == 0 && oo.extra_len == elens[b];
			/* a one-bit flip anywhere in the packet must be refused */
			if (good && l > 0) {
				pk2[l / 2] ^= 1;
				good = !seal_open(rsk, pk2, l, NULL, tok);
			}
			rt_ok &= good;
			rt_n++;
		}
	}
	CHECK(rt_ok, "C round-trip sweep: %d (plaintext,header) size combinations seal->open->tamper-reject", rt_n);
}

static void h_attestation(void)
{
	const jv *root = load_json("attestation.json");
	const jv *tags = jfield(root, "tags");
	const jv *signers = jfield(root, "signers");
	const jv *kat = jfield(root, "kat");
	const jv *neg = jarr(root, "negatives");
	const jv *exp = jarr(root, "expiry");
	const jv *ord = jarr(root, "ordering");
	const jv *sup = jarr(root, "supersedure");
	cur->expected_vectors = (int)(1 + neg->n + exp->n + ord->n + sup->n);

	CHECK(strcmp(jstr(tags, "node"), ATT_NODE_TAG) == 0, "node tag is %s", ATT_NODE_TAG);
	CHECK(strcmp(jstr(tags, "transport"), ATT_TRANSPORT_TAG) == 0, "transport tag is %s", ATT_TRANSPORT_TAG);
	keyset ka, kb;
	keyset_from_mnemonic(&ka, jstr(jfield(signers, "a"), "mnemonic"));
	keyset_from_mnemonic(&kb, jstr(jfield(signers, "b"), "mnemonic"));
	expect_hex("signer a", "node_id", ka.pk, 32, jstr(jfield(signers, "a"), "node_id"));
	expect_hex("signer b", "node_id", kb.pk, 32, jstr(jfield(signers, "b"), "node_id"));

	/* KAT: reconstruct both signed inputs from the wire fields, re-sign node leg */
	cur->vectors++;
	uint8_t *wire, *si_node, *si_tr, nsig[64];
	size_t wn = (size_t)jhex(kat, "wire", &wire);
	size_t sinn = (size_t)jhex(kat, "signed_input_node", &si_node);
	size_t sitn = (size_t)jhex(kat, "signed_input_transport", &si_tr);
	jhexn(kat, "node_sig", nsig, 64);
	CHECK(wn == ATT_LEN, "kat: wire is %d bytes", ATT_LEN);
	uint8_t in[74];
	size_t inn = att_node_input(in, wire);
	uint8_t want_in[sizeof ATT_NODE_TAG + 74];
	memcpy(want_in, ATT_NODE_TAG, strlen(ATT_NODE_TAG));
	memcpy(want_in + strlen(ATT_NODE_TAG), in, inn);
	CHECK(sinn == strlen(ATT_NODE_TAG) + inn && memcmp(si_node, want_in, sinn) == 0,
	      "kat: signed_input_node == tag || version || node || transport_key || transport_id || expiry");
	uint8_t want_tr[sizeof ATT_TRANSPORT_TAG + 32];
	memcpy(want_tr, ATT_TRANSPORT_TAG, strlen(ATT_TRANSPORT_TAG));
	memcpy(want_tr + strlen(ATT_TRANSPORT_TAG), wire + 1, 32);
	CHECK(sitn == strlen(ATT_TRANSPORT_TAG) + 32 && memcmp(si_tr, want_tr, sitn) == 0,
	      "kat: signed_input_transport == transport tag || node_pubkey");
	CHECK(memcmp(wire + 74, nsig, 64) == 0, "kat: wire carries node_sig at offset 74");
	CHECK(memcmp(wire + 1, ka.pk, 32) == 0, "kat: node_pubkey is signer a");
	uint8_t sig2[64];
	sign_tagged(sig2, ka.sk, ATT_NODE_TAG, in, inn);
	CHECK(memcmp(sig2, nsig, 64) == 0, "kat: C re-sign of node leg == frozen node_sig");
	char tok[96];
	att_node_leg(wire, wn, tok);
	CHECK(strcmp(tok, "ok") == 0, "kat: node leg verifies under strict predicate (got %s)", tok);

	/* every attestation wire in the file: the node leg must give exactly the
	 * expected outcome for the stages C owns (length, version, tid, node-sig);
	 * for entries whose expected failure is LATER (transport leg, expiry,
	 * supersedure/store) or that succeed, the node leg must itself accept. */
	static const char *const secs[3] = { "negatives", "ordering", "supersedure" };
	const jv *arrs[3] = { neg, ord, sup };
	int later_stage = 0;
	for (int s = 0; s < 3; s++) {
		for (size_t i = 0; i < arrs[s]->n; i++) {
			const jv *v = arrs[s]->kid[i];
			cur->vectors++;
			uint8_t *w;
			size_t n = (size_t)jhex(v, "wire", &w);
			const char *expect = jstr(v, "expect");
			att_node_leg(w, n, tok);
			int c_owned = strncmp(expect, "err:length:", 11) == 0 || strncmp(expect, "err:version:", 12) == 0 ||
			              strncmp(expect, "err:tid:", 8) == 0 || strncmp(expect, "err:node-sig:", 13) == 0;
			if (c_owned) {
				CHECK(strcmp(tok, expect) == 0, "%s[%zu] %s: got %s want %s", secs[s], i, jstr(v, "name"), tok, expect);
			} else {
				later_stage++;
				CHECK(strcmp(tok, "ok") == 0, "%s[%zu] %s: node leg must accept (expect \"%s\" is decided later); got %s",
				      secs[s], i, jstr(v, "name"), expect, tok);
			}
		}
	}
	/* expiry[]: a mesh-epoch policy comparison on one KAT, no crypto involved --
	 * deliberately not executed by the C harness; counted as skipped. */
	cur->vectors += (int)exp->n;
	cur->skipped += (int)exp->n;
	printf("  [attestation.json] node-leg executed for %d entries whose expected outcome is decided at a later stage "
	       "(transport leg = Layer 7 seam, expiry, supersedure); expiry[] x%zu skipped (epoch policy, no crypto)\n",
	       later_stage, exp->n);
}

/* ------------------------------------------------------------------------ */
/* supplement (non-frozen, harness-local): cofactor-class edges              */
/* ------------------------------------------------------------------------ */

static int g_supp_rows, g_supp_bites, g_supp_redundant, g_supp_resid, g_supp_resid_raw, g_supp_resid_wrapped;

static void h_supplement(const char *path)
{
	const char *saved = g_dir;
	char dir[1024], name[256];
	const char *slash = strrchr(path, '/');
	if (slash) {
		snprintf(dir, sizeof dir, "%.*s", (int)(slash - path), path);
		snprintf(name, sizeof name, "%s", slash + 1);
	} else {
		snprintf(dir, sizeof dir, ".");
		snprintf(name, sizeof name, "%s", path);
	}
	g_dir = dir;
	const jv *root = load_json(name);
	g_dir = saved;
	const jv *vs = jarr(root, "vectors");
	cur->expected_vectors = (int)vs->n;
	for (size_t i = 0; i < vs->n; i++) {
		const jv *v = vs->kid[i];
		cur->vectors++;
		uint8_t pk[32], sig[64], *msg;
		jhexn(v, "public_key", pk, 32);
		jhexn(v, "signature", sig, 64);
		size_t mn = (size_t)jhex(v, "message", &msg);
		const char *kind = jstr(v, "kind"), *vname = jstr(v, "name"), *ovar = jstr(v, "oracle_variant");
		int raw = crypto_ed25519_check(sig, pk, msg, mn) == 0;
		verdict vd = strict_verify(sig, pk, msg, mn);
		g_supp_rows++;
		if (strcmp(kind, "control-valid") == 0) {
			CHECK(raw && vd == V_OK && strcmp(ovar, "ok") == 0, "%s: honest control accepted by raw and wrapped", vname);
		} else if (strcmp(kind, "wrapper-bites") == 0) {
			g_supp_bites++;
			CHECK(raw, "%s: raw crypto_ed25519_check ACCEPTS this validly-signed input", vname);
			CHECK(strcmp(verdict_name(vd), ovar) == 0, "%s: wrapper rejects with %s (got %s)", vname, ovar, verdict_name(vd));
		} else if (strcmp(kind, "wrapper-redundant") == 0) {
			g_supp_redundant++;
			CHECK(strcmp(verdict_name(vd), ovar) == 0, "%s: wrapper rejects with %s (got %s; raw %s)", vname, ovar,
			      verdict_name(vd), raw ? "accepts" : "rejects");
		} else if (strcmp(kind, "residual-mixed-order") == 0) {
			/* strict oracle REJECTS; a cofactored verifier accepts. Until spec v2.1 this was
			 * an open residual (reported, never failed). v2.1 makes A and R torsion-free
			 * ([l]P = O), so the wrapper MUST reject: the row is now a gate. */
			g_supp_resid++;
			g_supp_resid_raw += raw;
			g_supp_resid_wrapped += (vd == V_OK);
			printf("  KNOWN-DELTA %s: oracle=%s raw Monocypher=%s wrapped=%s%s\n", vname, strcmp(ovar, "ok") ? "reject" : "accept",
			       raw ? "ACCEPT" : "reject", vd == V_OK ? "ACCEPT" : "reject", vd == V_OK ? "  [OPEN: wrapper does not close it]" : "  [closed]");
			CHECK(strcmp(jstr(v, "oracle"), "reject") == 0, "%s: supplement oracle says reject", vname);
			CHECK(strcmp(verdict_name(vd), ovar) == 0, "%s: v2.1 wrapper rejects with %s (got %s)", vname, ovar, verdict_name(vd));
		} else {
			check_(0, "%s: unknown kind \"%s\"", vname, kind);
		}
	}
}

/* ------------------------------------------------------------------------ */
/* driver                                                                    */
/* ------------------------------------------------------------------------ */

typedef struct {
	const char *name;
	void (*fn)(void);
} handler;

static const handler HANDLERS[] = {
	{ "(built-in primitive KATs)", h_selftest },
	{ "ed25519-rfc8032.json", h_ed25519 },
	{ "x25519-rfc7748.json", h_x25519 },
	{ "slip0010-published.json", h_slip0010_published },
	{ "slip0010-project-paths.json", h_slip0010_project },
	{ "keyset-project.json", h_keyset_project },
	{ "edge-vectors.json", h_edge },
	{ "announce.json", h_announce },
	{ "sealed-packet.json", h_sealed },
	{ "attestation.json", h_attestation },
};
#define NHANDLERS (sizeof HANDLERS / sizeof *HANDLERS)

static const char *const NOT_APPLICABLE[][2] = {
	{ "encoding.json", "hex/bech32/label presentation encodings; no Monocypher primitive involved" },
	{ "weak-key.json", "section 4.5 keygen-time pattern self-test (policy on key bytes); no Monocypher primitive" },
	{ "backup-format.json", "section 8/13 backup framing, NFKC hygiene, descriptor policy; no Monocypher primitive" },
	{ "restore-counters.json", "section 6.2/6.5 counter state-machine arithmetic; no Monocypher primitive" },
	{ "d9-migration.json", "Layer-1 directory migration/quarantine; no Monocypher primitive" },
};
#define NNA (sizeof NOT_APPLICABLE / sizeof *NOT_APPLICABLE)

int main(int argc, char **argv)
{
	const char *supplement = NULL;
	int list = 0;
	for (int i = 1; i < argc; i++) {
		if (strcmp(argv[i], "--list") == 0) list = 1;
		else if (strcmp(argv[i], "-v") == 0) g_verbose = 1;
		else if (strcmp(argv[i], "--supplement") == 0 && i + 1 < argc) supplement = argv[++i];
		else if (argv[i][0] != '-') g_dir = argv[i];
		else die("usage: harness [-v] [--list] [--supplement FILE] <test-vectors-dir>");
	}
	if (list) {
		for (size_t i = 0; i < NHANDLERS; i++)
			if (strstr(HANDLERS[i].name, ".json")) printf("consumed %s\n", HANDLERS[i].name);
		for (size_t i = 0; i < NNA; i++) printf("n/a %s\n", NOT_APPLICABLE[i][0]);
		return 0;
	}
	init_tables();
	torsion_tables();

	fstat st[NHANDLERS + 1];
	memset(st, 0, sizeof st);
	int total_fail = 0;
	for (size_t i = 0; i < NHANDLERS; i++) {
		st[i].name = HANDLERS[i].name;
		cur = &st[i];
		HANDLERS[i].fn();
		xfree_all();
		if (cur->vectors != cur->expected_vectors) {
			check_(0, "inventory: executed %d entries but the file has %d (no vector may be silently skipped)", cur->vectors,
			       cur->expected_vectors);
		}
		total_fail += cur->fail;
	}
	size_t nst = NHANDLERS;
	if (supplement) {
		st[nst].name = "supplement/cofactor-edges.json (non-frozen)";
		cur = &st[nst];
		h_supplement(supplement);
		xfree_all();
		if (cur->vectors != cur->expected_vectors)
			check_(0, "inventory: executed %d entries but the file has %d", cur->vectors, cur->expected_vectors);
		total_fail += cur->fail;
		nst++;
	}

	printf("\n%-42s %7s %7s %6s %5s %5s  %s\n", "FILE", "VECTORS", "CHECKS", "PASS", "FAIL", "SKIP", "RESULT");
	int tv = 0, tc = 0, tp = 0, tf = 0, ts = 0;
	for (size_t i = 0; i < nst; i++) {
		printf("%-42s %7d %7d %6d %5d %5d  %s\n", st[i].name, st[i].vectors, st[i].checks, st[i].pass, st[i].fail, st[i].skipped,
		       st[i].fail ? "FAIL" : "PASS");
		tv += st[i].vectors;
		tc += st[i].checks;
		tp += st[i].pass;
		tf += st[i].fail;
		ts += st[i].skipped;
	}
	printf("%-42s %7d %7d %6d %5d %5d  %s\n", "TOTAL", tv, tc, tp, tf, ts, tf ? "FAIL" : "PASS");
	for (size_t i = 0; i < NNA; i++) printf("%-42s %s -- %s\n", NOT_APPLICABLE[i][0], "n/a", NOT_APPLICABLE[i][1]);

	printf("\nSTRICT-PREDICATE DELTA (edge-vectors.json ed25519_strict): raw crypto_ed25519_check vs section-3 wrapper vs oracle\n");
	printf("  %-4s %-62s %-6s %-6s %-8s %s\n", "#", "vector", "oracle", "raw", "wrapped", "rejecting leg");
	int shown = 0, delta = 0;
	for (int i = 0; i < g_nedge; i++) {
		edge_row *r = &g_edge[i];
		if (r->raw_ok != r->oracle_ok) {
			delta++;
			printf("  %-4d %-62.62s %-6s %-6s %-8s %s\n", i, r->name, r->oracle_ok ? "ok" : "reject",
			       r->raw_ok ? "ACCEPT" : "reject", r->wrapped_ok ? "ACCEPT" : "reject", verdict_name(r->v));
			shown++;
		}
	}
	printf("  -> %d of %d vectors: raw Monocypher verdict differs from the oracle (every one is repaired by the wrapper)\n", delta,
	       g_nedge);
	printf("X25519 DELTA (edge-vectors.json x25519_low_order): %d low-order inputs; raw crypto_x25519 returned all-zero for %d "
	       "and a non-zero value for %d (Monocypher 4.x has no native rejection; ingress blacklist + all-zero abort are mandatory)\n",
	       g_x_low_total, g_x_low_raw_zero, g_x_low_raw_nonzero);
	if (supplement)
		printf("SUPPLEMENT (cofactor-edges.json, non-frozen): %d rows = 1 control + %d wrapper-bites + %d wrapper-redundant + %d residual-mixed-order;\n"
		       "  residual rows: raw Monocypher accepted %d/%d, wrapper accepted %d/%d (strict oracle rejects all) -> %s\n",
		       g_supp_rows, g_supp_bites, g_supp_redundant, g_supp_resid, g_supp_resid_raw, g_supp_resid, g_supp_resid_wrapped,
		       g_supp_resid, g_supp_resid_wrapped ? "OPEN residual" : "CLOSED (spec v2.1 torsion-free A/R leg)");
	(void)shown;
	free(g_allocs);
	return total_fail ? 1 : 0;
}
