// Regenerates a synthetic Letter Sealing V2 interoperability fixture using only
// Go's standard crypto libraries. No LINE WASM or translated crypto is used.
package main

import (
	"crypto/aes"
	"crypto/cipher"
	"crypto/ecdh"
	"crypto/hmac"
	"crypto/sha256"
	"encoding/base64"
	"encoding/binary"
	"encoding/json"
	"fmt"
	"os"
)

func must[T any](v T, err error) T {
	if err != nil {
		panic(err)
	}
	return v
}
func main() {
	if len(os.Args) != 3 {
		panic("usage: go run tools/generate-fixture.go exported-test-key.json fixture.json")
	}
	var fixture map[string]any
	if err := json.Unmarshal(must(os.ReadFile(os.Args[1])), &fixture); err != nil {
		panic(err)
	}
	pub := must(ecdh.X25519().NewPublicKey(must(base64.StdEncoding.DecodeString(fixture["public_key"].(string)))))
	raw := make([]byte, 32)
	for i := range raw {
		raw[i] = byte(i + 1)
	}
	private := must(ecdh.X25519().NewPrivateKey(raw))
	shared := must(private.ECDH(pub))
	fixture["shared_secret"] = base64.StdEncoding.EncodeToString(shared)
	derived := sha256.Sum256(append(append([]byte("salt"), shared...), []byte("info")...))
	macHash := hmac.New(sha256.New, derived[:])
	macHash.Write([]byte("probe"))
	primitiveBlock := must(aes.NewCipher(derived[:]))
	primitiveGCM := must(cipher.NewGCM(primitiveBlock))
	primitiveNonce := []byte{0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11}
	primitiveCiphertext := primitiveGCM.Seal(append([]byte{}, primitiveNonce...), primitiveNonce, []byte("probe"), nil)
	fixture["primitives"] = map[string]string{
		"derived_key":    base64.StdEncoding.EncodeToString(derived[:]),
		"hmac":           base64.StdEncoding.EncodeToString(macHash.Sum(nil)),
		"aes_ciphertext": base64.StdEncoding.EncodeToString(primitiveCiphertext),
	}
	salt := []byte("fixed-test-salt!")
	if len(salt) != 16 {
		panic("V2 salt must be 16 bytes")
	}
	hash := sha256.New()
	hash.Write(shared)
	hash.Write(salt)
	hash.Write([]byte("Key"))
	key := hash.Sum(nil)
	nonce := make([]byte, 12)
	binary.BigEndian.PutUint64(nonce, 0x100000001)
	copy(nonce[8:], []byte{1, 2, 3, 4})
	to := "u8ae764e8e69e6bd4ecdd9b6ea0c40fce"
	from := "uf1a2b3c4d5e6f7a8b9c0d1e2f3a4b5c6"
	aad := append([]byte(to), []byte(from)...)
	for _, n := range []uint32{42, 99, 2, 0} {
		aad = binary.BigEndian.AppendUint32(aad, n)
	}
	plaintext := []byte(`{"text":"Independent Go ciphertext -> original LINE WASM: 日本語 🔐"}`)
	block := must(aes.NewCipher(key))
	gcm := must(cipher.NewGCM(block))
	ciphertext := append(append([]byte{}, salt...), nonce...)
	ciphertext = gcm.Seal(ciphertext, nonce, plaintext, aad)
	// Independently encrypt the V1 format as an additional check on ECDH.
	salt1 := []byte("salt1234")
	hash.Reset()
	hash.Write(shared)
	hash.Write(salt1)
	hash.Write([]byte("Key"))
	key1 := hash.Sum(nil)
	hash.Reset()
	hash.Write(shared)
	hash.Write(salt1)
	hash.Write([]byte("IV"))
	ivhash := hash.Sum(nil)
	iv := make([]byte, 16)
	for i := range iv {
		iv[i] = ivhash[i] ^ ivhash[i+16]
	}
	block1 := must(aes.NewCipher(key1))
	pad := 16 - len(plaintext)%16
	padded := append([]byte{}, plaintext...)
	for range pad {
		padded = append(padded, byte(pad))
	}
	encrypted1 := make([]byte, len(padded))
	cipher.NewCBCEncrypter(block1, iv).CryptBlocks(encrypted1, padded)
	digest := sha256.Sum256(encrypted1)
	mac := make([]byte, 16)
	for i := range mac {
		mac[i] = digest[i] ^ digest[i+16]
	}
	block1.Encrypt(mac, mac)
	ciphertext1 := append(append(append([]byte{}, salt1...), encrypted1...), mac...)
	fixture["ciphertext_v1"] = base64.StdEncoding.EncodeToString(ciphertext1)
	fixture["peer_public_key"] = base64.StdEncoding.EncodeToString(private.PublicKey().Bytes())
	fixture["ciphertext"] = base64.StdEncoding.EncodeToString(ciphertext)
	fixture["plaintext"] = string(plaintext)
	fixture["to"] = to
	fixture["from"] = from
	fixture["generator"] = "Go standard library crypto/ecdh + crypto/aes + crypto/cipher; fixed synthetic peer private key bytes 1..32"
	if err := os.WriteFile(os.Args[2], append(must(json.MarshalIndent(fixture, "", "  ")), '\n'), 0644); err != nil {
		panic(err)
	}
	fmt.Printf("Wrote independent V2 ciphertext fixture (%d bytes)\n", len(ciphertext))
}
