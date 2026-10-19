# T9 native crypto trial

The message primitives are portable. A complete replacement for `ltsm.wasm`
is **not established**: token loading, persisted private-key blobs and the MID
key still depend on the proprietary SKB engine. This trial adds native
plaintext-key operations and differential tests; every production caller still
uses `Runtime`. The account net worker owns one runtime and calls it directly for live decode, history and signing; the blob remains required.

Evidence is pinned to Chrome 3.7.2, SHA-256
`58bb4e189ab9bbd7d72ed415258da36afd50e306cb019bab75329d4f5f1b65b3`.
Only the committed synthetic fixtures, generated test inputs, the public
application token in `src/lib.rs`, and immutable module data were used. No
account state, dumps, transport or UI was exercised.

## Called surface

`B` means a byte array passed through an emval handle, `S` an embind string,
and class names denote WASM object pointers. Byte results are emval handles
consumed by `Runtime::output`. Integers are unsigned 32-bit except the V2
sequence, which crosses the ABI as a full 64-bit integer. Inherited methods
are listed under the class name used by callers.

“Ported” below describes the plaintext-key primitive, not compatibility with
SKB serialized keys. Native encryption takes explicit randomness so the
caller controls nonce freshness and tests can compare complete frames.

| Entry point | Argument → result shape | Operation / caller | Primitive and status |
| --- | --- | --- | --- |
| `SecureKey.loadToken` | `S(application token)` → `SecureKey` | Request signing, `lib.rs` | **Blocked:** token decoding and SKB unwrap using embedded key material; not a plaintext key import. |
| `SecureKey.loadKey` | `B(wrapped key)` → `SecureKey` | Synthetic opaque MID construction, `main.rs` | **Blocked:** SKB blob loading. The embedded MID blob is 193 bytes. |
| `SecureKey.deriveKey` | `(B(prefix), B(suffix))` → `SecureKey` | Version/access-token binding before request HMAC, `lib.rs` | **Ported:** SHA-256(prefix ‖ key ‖ suffix), with nonempty inputs. Resulting SKB object serialization is not ported. |
| `Hmac` constructor | `SecureKey` → `Hmac` | Request signing, `lib.rs` | **Ported:** native HMAC takes raw key bytes. |
| `Hmac.digest` | `B(message)` → `B(32)` | Request signing, `lib.rs` | **Ported:** HMAC-SHA256. |
| `Curve25519Key.generate` | `bool(exportable = true)` → `Curve25519Key` | QR pairing; synthetic checks, `client.rs`, `main.rs` | X25519 key generation inside SKB. **Ported:** raw public-key derivation with caller-provided scalar bytes; **blocked:** matching export policy/blob generation. |
| `Curve25519Key.loadKey` | `B(exported key)` → `Curve25519Key` | Restore pairing key, fixture checks, `client.rs`, `main.rs`, `client/chat.rs` | **Blocked:** SKB import followed by X25519 key setup. Fixture input is 193 bytes. |
| `Curve25519Key.getPublicKey` | `()` → `B(32)` | Pairing public key / restore checks, same files | X25519 scalar multiplication by base point. **Ported:** `x25519_public_key` over caller-provided scalar bytes. Private scalar access through SKB import remains **blocked**. |
| `Curve25519Key.exportKey` | `()` → `B(wrapped key)` | Persist pairing key / synthetic fixture export, `client.rs`, `main.rs` | Inherited `SecureKey.exportKey`; **blocked:** proprietary export blob, not raw scalar bytes. |
| `Curve25519Key.createChannel` | `B(peer public key, 32)` → `E2EEChannel` | Phone key transfer / synthetic message checks, same files | Raw X25519 is **ported** with `x25519`; `Channel::from_x25519` matches curve-point ECDH and low-order acceptance/encoding. Twist-point semantics diverge (see below). SKB private-key loading remains **blocked**. |
| `E2EEKey.loadKey` | `B(serialized E2EE key)` → `E2EEKey` | Restore Letter Sealing keys, `client.rs`, `client/chat.rs` | **Blocked:** E2EE metadata envelope plus SKB private-key import. Exact envelope not established in this trial. |
| `E2EEKey.getPublicKey` | `()` → `B(32)` | Key restore checks / group sender, `client.rs` | Inherited X25519 public-key operation; same status as `Curve25519Key.getPublicKey`. |
| `E2EEKey.createChannel` | `B(peer public key, 32)` → `E2EEChannel` | Direct/group message channels, `client/chat.rs` | Inherited X25519 ECDH; same status as `Curve25519Key.createChannel`. |
| `E2EEKey.exportKey` | `()` → `B(serialized E2EE key)` | Persist transferred keys, `client.rs` | **Blocked:** metadata plus wrapped key; no raw scalar export. |
| `E2EEKey.getKeyId` | `()` → `u32` | Persist transferred key identity, `client.rs` | Metadata accessor, **portable-not-done** with the blocked E2EE envelope. |
| `E2EEKey.getVersion` | `()` → `u32` | Persist transferred key version, `client.rs` | Metadata accessor, **portable-not-done** with the blocked E2EE envelope. |
| `E2EEKeychain.size` | `()` → `u32` | Bound transferred-key iteration, `client.rs` | Vector accessor, **portable-not-done** with keychain decoding. |
| `E2EEKeychain.get` | `u32(index)` → emval object pointer (`E2EEKey`) | Persist each transferred key, `client.rs` | Vector accessor, **portable-not-done** with keychain decoding; `Runtime::object` consumes the handle. |
| `E2EEChannel.unwrapE2EEKeyChain` | `B(encrypted phone keychain)` → `E2EEKeychain` | Pairing, `client.rs` | **Blocked:** SKB key unwrap plus keychain parsing. Disassembly derives the unsalted `Key` and `IV` values used by V1; complete keychain layout/padding/authentication not established by successful synthetic vectors. |
| `E2EEChannel.unwrapGroupSharedKey` | `B(encrypted group key)` → `E2EEKey` | Existing group messages/sends, `client/chat.rs`, `client/desktop.rs` | **Blocked:** SKB unwrap into an X25519 key object. Disassembly derives unsalted `Key` and folded `IV`; exact keyblob mode/framing not proven. |
| `E2EEChannel.encryptV1` | `B(plaintext)` → `B(salt8 ‖ CBC ‖ MAC16)` | Direct/group sends, `client/desktop.rs`; `main.rs` checks | **Ported:** SHA-256 schedule, AES-256-CBC, PKCS#7, folded-hash/AES MAC. |
| `E2EEChannel.decryptV1` | `B(frame)` → `B(plaintext)` | Direct/group history, `client/chat.rs`; `main.rs` checks | **Ported:** authenticated V1 decryption and padding validation. |
| `E2EEChannel.encryptV2` | `(S(to), S(from), u32(sender key), u32(receiver key), u32(content), u64(sequence), B(plaintext))` → `B(salt16 ‖ nonce12 ‖ ciphertext ‖ tag16)` | Direct/group sends, `client/desktop.rs`; `main.rs` checks | **Ported:** SHA-256 schedule, AES-256-GCM; native context takes decoded MIDs. Opaque MID decoding is **blocked** on its embedded SKB key. |
| `E2EEChannel.decryptV2` | Same context without sequence, `B(frame)` → `B(plaintext)` | Direct/group history, `client/chat.rs`; `main.rs` checks | **Ported:** authenticated V2 decryption; same MID boundary. |
| `AesKey.generate` | `(u32(length = 32), B(empty options))` → `AesKey` | Synthetic AES checks, `main.rs` | **Portable-not-done:** random-key/object creation. The AES-256 primitive is ported; no native RNG or SKB export is added. |
| `AesKey` constructor | `SecureKey` → `AesKey` | Synthetic opaque MID creation, `main.rs` | **Ported:** raw 256-bit key binding. The actual MID key import is **blocked**. |
| `AesKey.encrypt` | `B(plaintext)` → `B(nonce12 ‖ ciphertext ‖ tag16)` | Synthetic AES/MID checks, `main.rs` | **Ported:** AES-256-GCM, no AAD. |
| `AesKey.decrypt` | `B(frame)` → `B(plaintext)` | Synthetic AES checks, `main.rs` | **Ported:** AES-256-GCM, no AAD; matches the oracle's empty-frame rejection. |

The new tests additionally construct `Curve25519Key(SecureKey)` to bind a
known scalar and `E2EEChannel(SecureKey)` to bind a known derived secret,
and call `SecureKey.exportKey()` to test the opaque boundary. These
correspond to raw curve operations and native `Channel::new`; opaque export
remains blocked.
`destroy`, handle release, string allocation and `Runtime::object` are host
lifetime operations, not cryptographic primitives. Runtime initialization
also calls exports `P`, `T` and the registered global `isAvailable`.

The binding catalog contains other functions (`wrapGroupSharedKey`,
`generateHashKeyChainToConfirmE2EE`, identity-V3 unwrap, timestamps and vector
mutators), but the existing Rust client does not call them. They are not
claimed as ported. An exploratory call to `wrapGroupSharedKey` with 32 raw
bytes failed with `Invalid EC key length`; its input is not a plaintext
group scalar.

Media does **not** call the WASM in this client. It is already native in
`src/client/desktop/assets.rs`: HKDF-SHA256, empty salt, info `FileEncryption`,
76 output bytes = AES key32 ‖ HMAC key32 ‖ nonce12. AES-256-CTR starts at
`nonce12 ‖ BE32(0)`, incrementing the low 32 bits. The stored frame is
ciphertext ‖ HMAC-SHA256(ciphertext). HMAC is verified before decrypting;
there is no padding or AAD. The independent `fixtures/media.json` and its
existing unit tests remain unchanged. No WASM media oracle is claimed.

## Proven constructions

Let `Z` be the raw 32-byte X25519 shared secret and `fold(H)` XOR the two
16-byte halves of a SHA-256 digest.

For V1, choose a fresh 8-byte salt:

```text
K  = SHA256(Z || salt || "Key")
IV = fold(SHA256(Z || salt || "IV"))
C  = AES256-CBC(K, IV, PKCS7(plaintext))
M  = AES256-ECB(K, fold(SHA256(C)))
frame = salt[8] || C[positive multiple of 16] || M[16]
```

V1 has no AAD. Native decryption checks the MAC before decrypting and checks
all PKCS#7 padding bytes. The empty plaintext is a full padding block. This
is the observed LINE construction, not HMAC or CMAC.

For V2, choose a fresh 16-byte salt and four random nonce bytes:

```text
K     = SHA256(Z || salt || "Key")
nonce = BE64(sequence) || random[4]
AAD   = UTF8(decoded_to) || UTF8(decoded_from)
        || BE32(sender_key_id) || BE32(receiver_key_id)
        || BE32(2) || BE32(content_type)
frame = salt[16] || nonce[12] || AES256-GCM(K, nonce, AAD, plaintext)
```

The GCM result is ciphertext followed by the full 16-byte tag, without
padding. For the ordinary 33-byte MIDs, AAD is 82 bytes. All 64 sequence bits
are preserved. The oracle accepts an empty V2 plaintext.

`AesKey` uses the same GCM primitive with no AAD, prepending the 12-byte
random nonce. It can encrypt an empty message into a 28-byte frame, but its
decrypt method rejects frames of length ≤28. The native wrapper deliberately
matches this. `SecureKey.deriveKey` rejects an empty prefix or suffix through
the existing runtime; native `derive_key` does too. Neither restriction is a
limitation of SHA-256 or GCM themselves.

For signing, the known outer construction is:

```text
D = SHA256(SHA256(version) || token_key || SHA256(access_token))
signature = base64(HMAC-SHA256(D, UTF8(path || body)))
```

The `token_key` extraction from `TOKEN` is unresolved. This trial therefore
does not claim a native replacement for `sign`.

## SecureKey evidence and feasibility boundary

`SecureKey` is an **opaque, policy-bearing SKB key**, not a plain Rust-style
byte container at the exposed API. Plaintext key bytes are not returned by
the tested public bindings:

* `secure_key_boundary` asserts rejection of synthetic 0/16/32/48/64/193-byte
  arrays: 0/16 bytes report `invalid parameters`, 32 bytes `invalid format`,
  and 48/64/193 bytes of `0x42` report `operation not supported`. The test
  asserts these error substrings, including their spelling. A valid 193-byte
  fixture or embedded MID blob loads successfully.
* The application token loads and supports derivation/HMAC, but exporting
  that token key raises `illegal operation`. A derived key also refuses
  export, even when its bytes are independently known. `secure_key_boundary`
  asserts `illegal operation` for token export, derived-key export, and
  export through `Curve25519Key(SecureKey)` around the derived key; that
  wrapper does not bypass the export policy.
* Exportable synthetic curve keys persist as wrapped blobs, not 32-byte
  private scalars. `secure_key_boundary` generates an exportable curve key,
  asserts its export is 193 bytes, destroys the original, imports the blob
  in a fresh runtime, and asserts public-key equality.
* The module includes SKB engine/error/cache/export-version machinery. In
  `wasm-tools print` output, function 715 (`loadToken` implementation) loads
  embedded data at address 7776, length 165, then uses SKB derivation/unwrap
  calls. Function 217 dispatches `deriveKey` through the engine; functions
  163/201 build the message key/IV schedules. These indices apply only to
  the pinned blob. They do not by themselves identify the engine's internal
  cipher or white-box encoding.

This establishes the opaque API boundary, **not** that the plaintext can
never exist in linear memory or that extraction is cryptographically
impossible. No audited white-box/keyblob decoder was recovered here. A full
port requires that decoder, token-key recovery, MID decoding and key-transfer
formats; changing the AES implementation alone cannot remove the blob.

The tests cross the boundary without reading private-key memory. The Go
fixture generator computes `Z` from its fixed synthetic peer scalar (bytes
1..32) and the fixture public key. Another experiment uses the X25519 public
base point as peer: the resulting channel secret equals the key's public
key, and native HMAC with that known value matches WASM HMAC. Derived keys
can then vary deterministically, and their bytes are checked independently
through HMAC and AES operations.

The local cache now contains `aes-gcm` 0.10.3, `x25519-dalek` 2.0.1
(`static_secrets`) and `curve25519-dalek` 4.1.3. Dependencies resolve offline.
AES-GCM uses the RustCrypto AEAD implementation; the handwritten GHASH and
GCM counter/tag code have been removed. `subtle` remains necessary for V1
MAC comparison and constant-time selection of the low-order channel secret.
`curve25519-dalek` is a direct test dependency for classifying arbitrary
points, and a transitive production dependency of `x25519-dalek`.

Raw public-key derivation and RFC 7748 X25519 are exposed as
`x25519_public_key` and `x25519`. A known scalar can be bound to the oracle
with `Curve25519Key(SecureKey.deriveKey(...))`: its bytes are independently
computed and checked by HMAC, and its public key is compared directly.
This makes differential testing possible without reading private memory or
recovering the SKB decoder. It does not enable loading arbitrary scalar
bytes into the oracle, so exact RFC scalar vectors run natively; RFC scalars
also participate as peers in the differential ECDH checks.

The oracle accepts all seven tested low-order encodings (0, 1, the two
order-8 encodings, p−1, p, p+1), without rejecting field-zero shared results.
Its channel secret is encoded as p = `edff…ff7f`, not canonical 32-byte zero.
The tests assert acceptance through HMAC and successful native-to-oracle V1
decryption. Raw `x25519` returns canonical zero as RFC 7748 requires;
`Channel::from_x25519` maps that result to p in constant time for oracle
compatibility. The distinction matters for the subsequent hash schedule.

There is an additional compatibility limit: the oracle accepts twist
u-coordinates (`createChannel` succeeds), but produces a different secret
from RFC X25519. The secret depends on the scalar. A seeded 64-point test
asserts 43 curve-point matches and 21 twist-point divergences through HMAC,
classifying points using dalek's checked Edwards conversion.

Independent round-2 review additionally established that the twist secret
matches none of these candidate constructions: zero, the p-encoding, u;
X25519 of −u or of unmasked u reduced mod p, or negated X25519 output;
affine Weierstrass or Montgomery (B=1) arithmetic with an off-curve square-root
y; or Edwards arithmetic with an off-curve x, using either unified formulas
or dedicated doubling. These exclusions and the scalar dependence are
reviewer findings, not assertions added to this trial's test suite. The
review also confirmed that bit 255 is masked and noncanonical u is reduced
mod p, as RFC 7748 specifies.

Native parity for twist inputs would require reverse-engineering SKB point
arithmetic. Honest peers send curve points, so this divergence does not
affect interoperability; a twist key would fail decryption. The raw native
implementation follows
[RFC 7748](https://www.rfc-editor.org/rfc/rfc7748.html#section-5.2), including
twist inputs; no unchecked or handwritten curve arithmetic was added to
reproduce the proprietary behavior. `Channel::from_x25519` has proven oracle
parity for curve public keys and the tested low-order points, not arbitrary
twist inputs. This remains a trial API with no production callers. Full
arbitrary-input ECDH parity is not claimed.

## Differential coverage and reproduction

The unused T9 native-crypto trial, including its tests, is behind the
non-default `native-crypto` Cargo feature. Native Go, padding and RFC-vector
tests run with that feature without needing the WASM blob. Oracle suites are
explicitly ignored by the harness with a fixture requirement, so missing
coverage is reported as ignored rather than passed. To run an oracle suite:

```sh
cargo test --release --offline --features native-crypto aes_matches_oracle -- --ignored --nocapture
```

Explicit oracle runs require `artifacts/line-3.7.2/ltsm.wasm` or an explicit
`LINE_CRYPTO_ORACLE_PATH` pointing to that fixture. A missing fixture fails loudly with the named
`SKIP native crypto oracle` diagnostic; permission errors and wrong hashes
also fail. There is no successful early return for an absent oracle.

Four seeded suites each run 320 cases:

* Derivation/HMAC: varying prefix, suffix and message bytes; direct HMAC
  checks include empty messages. Derived bytes are checked through both
  HMAC and AES, and empty derivation arguments must be rejected by both.
* Curve: varying independently known derived scalars, seeded peer scalars
  and generated curve points. Public keys match directly; shared secrets
  match through HMAC and native-to-oracle V1 decrypt. The first four peers
  use the RFC 7748 section 5.2/6.1 scalar vectors. Separate tests cover the
  seven low-order points and the arbitrary-point boundary above.
* AES: varying derived keys, plaintext and explicit nonce. Oracle encryption
  is rebuilt byte for byte natively; native frames decrypt identically in
  WASM. Empty plaintext encryption and the decrypt rejection are checked.
* Letter Sealing: varying derived channel secrets, plaintext, key IDs,
  content type, salt, sequence and nonce. Each of V1 and V2 has 320
  byte-identical oracle/native encryption comparisons and both directions
  of decryption. Native-generated frames use seeded randomness independent
  of the oracle's random output.

The xorshift64 seed is `0x743963727970746f`. Lengths include 0, 1, 15, 16,
17, 31/32/33, 63/64/65, 255/256/257, 1024, 4096 and 65536 bytes, plus seeded
lengths through 2048. Sequence cases include zero, the 32-bit boundary and
`u64::MAX`. Oracle-generated salt/nonce bytes are copied only as explicit
inputs to reproduce that particular encryption; native-to-oracle decrypt
uses separate seeded salt/nonce values.

The Go fixture adds independently computed shared-secret, derive, HMAC and
AES-GCM vectors without changing its existing V1/V2 ciphertexts. Fixed-vector
tests check those plus both message formats. There are also 16 differential
rejections: nonce/salt, ciphertext and MAC/tag bit flips for all three
formats, both parts of the V2 nonce, and changes to each of the five V2
context fields. Each attempt first asserts that the same oracle instance
accepts the untampered frame. AES checks bind `AesKey` around the known
channel secret and call `AesKey.decrypt`; they cannot pass on a missing
`E2EEChannel.decrypt` binding. Failed WASM instances are discarded, never
reused.

Five additional frames are built natively from explicit padded CBC blocks
and freshly computed valid MACs, bypassing the production padding writer.
Short/inconsistent padding (`01 02` and `04 03 04 04`), pad 0, pad 17 and
pad 255 all fail the native padding check and oracle unpadding. Each oracle
instance first accepts a valid padding frame. These cases kill the formerly
surviving `&&`→`||` mutation; the native error assertion also proves that
MAC verification succeeded before padding rejection.

```sh
GOCACHE="$PWD/target/t9-fixture-go-cache" GOTELEMETRY=off GOPROXY=off \
  go run tools/generate-fixture.go fixtures/letter-sealing-v2.json fixtures/letter-sealing-v2.json
cargo test --release --offline -p line --features native-crypto native_crypto -- --nocapture
cargo test --release --offline -p line --features native-crypto native_crypto::tests::benchmark_decrypt -- --ignored --nocapture
```

The round-2 benchmark on this worktree measured a 74-byte fixture message, reused
channel, five oracle warmups, 100 oracle iterations and 10,000 native
iterations: **4623.446 µs oracle vs 0.407 µs native, 11354.8×**, with 455.2 ms
oracle setup (runtime, fixture key/channel and synthetic opaque IDs). Both
timed paths include result checking/allocation and use `black_box`. Native
timing starts with the raw secret and decoded MIDs; it excludes the unresolved
SKB/key/MID work. This is a local microbenchmark, not application latency.

Validation in this worktree (four gates rerun in the final round; benchmark
retained from round 2):

| Command | Result |
| --- | --- |
| `cargo fmt --all --check` | Passed. |
| `cargo clippy --release --offline --workspace --all-targets -- -D warnings` | Passed without warnings. |
| `cargo test --release --offline --workspace` | Passed: desktop 73 passed / 3 ignored; library 46 passed / 2 ignored; other binary and doc-test targets have no tests. The crypto tests contribute 14 passing tests (including 10 independent oracle suites) and one ignored benchmark. |
| `cargo build --release --offline -p line-desktop` | Passed. |
| Benchmark command above | Passed: one benchmark test executed explicitly. |

The second-round Clippy check requested array chunk iteration in the new
padding-frame builder; it was corrected before the final successful gates.
The formerly surviving padding mutant was rerun in isolation and killed by
`native_rejects_authenticated_invalid_v1_padding` (short padding was wrongly
accepted by the mutant). The original condition was restored before the
gates. The existing ignored tests were not enabled.

## Deliberate limits

No production call site, runtime implementation, thread ownership, transport
or UI changed. No native signing/token/keyblob fallback was installed.
AES-128/192, SKB serialization/white-box internals, group/keychain transfer, opaque MID conversion and RNG policy are not claimed as ported.
The existing native media path was not duplicated. Neither the problem
register nor the acceptance log was edited.

This is interoperability evidence for synthetic data, not a security audit.
Key lifetime/zeroization, arbitrary-input ECDH compatibility and wider
malformed-input behavior need review before any production switch. The
proprietary disassembly directory `target/crypto-research/` was deleted;
no WAT extracts are retained in the worktree. Nonce freshness is the caller's responsibility. The primary open question is how
to replace the SKB key and token boundary without shipping or bootstrapping
through the proprietary blob.
