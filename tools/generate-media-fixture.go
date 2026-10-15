// Generate an independent synthetic fixture for Chrome's encrypted photo format.
package main

import (
	"crypto/aes"
	"crypto/cipher"
	"crypto/hkdf"
	"crypto/hmac"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"os"
)

func main() {
	material := make([]byte, 32)
	for i := range material {
		material[i] = byte(i)
	}
	keys, err := hkdf.Key(sha256.New, material, nil, "FileEncryption", 76)
	if err != nil {
		panic(err)
	}
	iv := make([]byte, 16)
	copy(iv, keys[64:])
	block, err := aes.NewCipher(keys[:32])
	if err != nil {
		panic(err)
	}
	plaintext := []byte("Independent photo fixture: 日本語")
	ciphertext := make([]byte, len(plaintext))
	cipher.NewCTR(block, iv).XORKeyStream(ciphertext, plaintext)
	mac := hmac.New(sha256.New, keys[32:64])
	mac.Write(ciphertext)
	ciphertext = append(ciphertext, mac.Sum(nil)...)
	fixture := map[string]string{
		"generator":  "Go standard library HKDF-SHA256 + AES256-CTR + HMAC-SHA256; synthetic material 0..31",
		"material":   base64.StdEncoding.EncodeToString(material),
		"ciphertext": base64.StdEncoding.EncodeToString(ciphertext),
		"plaintext":  string(plaintext),
	}
	data, err := json.MarshalIndent(fixture, "", "  ")
	if err != nil {
		panic(err)
	}
	if err := os.WriteFile("fixtures/media.json", append(data, '\n'), 0644); err != nil {
		panic(err)
	}
}
