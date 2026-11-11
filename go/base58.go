package builddeed

import (
	"errors"
	"math/big"
	"strings"
)

// The Bitcoin base58 alphabet, which NEAR uses for keys and hashes.
const base58Alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"

var big58 = big.NewInt(58)

// base58Encode encodes b. Each leading zero byte becomes a leading "1".
func base58Encode(b []byte) string {
	n := new(big.Int).SetBytes(b)
	var out []byte
	mod := new(big.Int)
	for n.Sign() > 0 {
		n.DivMod(n, big58, mod)
		out = append(out, base58Alphabet[mod.Int64()])
	}
	for _, c := range b {
		if c != 0 {
			break
		}
		out = append(out, '1')
	}
	for i, j := 0, len(out)-1; i < j; i, j = i+1, j-1 {
		out[i], out[j] = out[j], out[i]
	}
	return string(out)
}

// base58Decode decodes s. Each leading "1" becomes a leading zero byte.
func base58Decode(s string) ([]byte, error) {
	n := new(big.Int)
	zeros := 0
	for zeros < len(s) && s[zeros] == '1' {
		zeros++
	}
	for i := 0; i < len(s); i++ {
		d := strings.IndexByte(base58Alphabet, s[i])
		if d < 0 {
			return nil, errors.New("not base58")
		}
		n.Mul(n, big58)
		n.Add(n, big.NewInt(int64(d)))
	}
	return append(make([]byte, zeros), n.Bytes()...), nil
}
