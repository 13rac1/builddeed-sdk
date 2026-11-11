package builddeed

// Agents buy and register with no browser. The owner deposits an agent
// budget with a full-access key (deposit_budget), and gives the agent a
// function-call key for buy_from_budget and add_device_from_budget only.

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"slices"
	"time"
)

// AgentBudget is an agent budget, as get_budget returns it. NEAR amounts are
// in yoctoNEAR.
type AgentBudget struct {
	// Balance is the NEAR that is left.
	Balance string `json:"balance"`
	// MaxPrice is the highest price of one license.
	MaxPrice string `json:"max_price"`
	// Products are the products that the budget can buy.
	Products []string `json:"products"`
}

// OwnedLicense is a license of the product, as licenses_for_owner returns it.
type OwnedLicense struct {
	// TokenID is the license token id, for example "checksum:1".
	TokenID string `json:"token_id"`
	// UpdatesUntilMs is the end of the update period, in Unix milliseconds.
	UpdatesUntilMs int64 `json:"updates_until_ms"`
	// Devices are the computer ids with a seat.
	Devices []string `json:"devices"`
}

// Purchase is the result of BuyWithKey.
type Purchase struct {
	// TokenID is the license token id.
	TokenID string
	// New is false when the account owned a current license already and
	// nothing was sent.
	New bool
}

// Budget returns the agent budget of accountID, or nil when it has none.
func (c *Client) Budget(ctx context.Context, accountID string) (*AgentBudget, error) {
	var b *AgentBudget
	if err := c.view(ctx, "get_budget", map[string]string{"account_id": accountID}, &b); err != nil {
		return nil, err
	}
	return b, nil
}

// Licenses returns the licenses of the product that accountID owns.
func (c *Client) Licenses(ctx context.Context, accountID string) ([]OwnedLicense, error) {
	var ls []OwnedLicense
	if err := c.view(ctx, "licenses_for_owner", map[string]string{"account_id": accountID, "product": c.product.Key}, &ls); err != nil {
		return nil, err
	}
	return ls, nil
}

// productInfo is the part of get_product that the agent calls use.
type productInfo struct {
	Price      string `json:"price"`
	MaxDevices int    `json:"max_devices"`
}

func (c *Client) productInfo(ctx context.Context) (productInfo, error) {
	var p *productInfo
	if err := c.view(ctx, "get_product", map[string]string{"key": c.product.Key}, &p); err != nil {
		return productInfo{}, err
	}
	if p == nil {
		return productInfo{}, fmt.Errorf("the product %q does not exist in %s", c.product.Key, c.product.Contract)
	}
	return *p, nil
}

// BuyWithKey buys a license of the product for the key's account, paid from
// the account's agent budget. key must be a function-call key for
// buy_from_budget and add_device_from_budget only. When the account owns a
// license of the product whose update period has not ended, BuyWithKey sends
// nothing and returns that license with New false, so a second run buys no
// second license. The call passes the price that it read, so the contract
// refuses it when the price changed.
func (c *Client) BuyWithKey(ctx context.Context, key KeyFile) (Purchase, error) {
	if key.AccountID == "" {
		return Purchase{}, ErrNoAccount
	}
	owned, err := c.Licenses(ctx, key.AccountID)
	if err != nil {
		return Purchase{}, err
	}
	now := time.Now().UnixMilli()
	for _, l := range owned {
		if l.UpdatesUntilMs > now {
			return Purchase{TokenID: l.TokenID}, nil
		}
	}
	p, err := c.productInfo(ctx)
	if err != nil {
		return Purchase{}, err
	}
	ret, txHash, err := c.call(ctx, key, "buy_from_budget", map[string]string{"product": c.product.Key, "price": p.Price})
	if err != nil {
		return Purchase{}, err
	}
	var bought struct {
		TokenID string `json:"token_id"`
	}
	if err := json.Unmarshal(ret, &bought); err != nil || bought.TokenID == "" {
		// The purchase ran, but its token id is unknown.
		return Purchase{}, &UnconfirmedError{TxHash: txHash, Err: fmt.Errorf("the buy_from_budget reply has no token id: %s", ret)}
	}
	return Purchase{TokenID: bought.TokenID, New: true}, nil
}

// RegisterWithKey gives this computer a seat on a license of the key's
// account, with no browser, and pays the storage from the account's agent
// budget. It returns the token id of the license. tokenID picks the license;
// with "", it takes the current license of the product with a free seat
// whose update period ends last. When this computer has a seat on a license
// of the account already, RegisterWithKey sends nothing: it returns that
// license, or an error when tokenID names another one.
//
// The seat is for the computer that runs this: the computer id comes from
// its machine id. Run it on the computer that will run the program, not in a
// container or a cloud sandbox.
func (c *Client) RegisterWithKey(ctx context.Context, key KeyFile, tokenID string) (string, error) {
	if key.AccountID == "" {
		return "", ErrNoAccount
	}
	device, err := c.DeviceID()
	if err != nil {
		return "", err
	}
	owned, err := c.Licenses(ctx, key.AccountID)
	if err != nil {
		return "", err
	}
	for _, l := range owned {
		if !slices.Contains(l.Devices, device) {
			continue
		}
		if tokenID != "" && tokenID != l.TokenID {
			return "", fmt.Errorf("this computer has a seat on %s already; remove it there first", l.TokenID)
		}
		return l.TokenID, nil
	}
	if tokenID == "" {
		if tokenID, err = c.freeLicense(ctx, key.AccountID, owned); err != nil {
			return "", err
		}
	}
	if _, _, err := c.call(ctx, key, "add_device_from_budget", map[string]string{"token_id": tokenID, "device_id": device}); err != nil {
		return "", err
	}
	return tokenID, nil
}

// freeLicense returns the license in owned with a free seat and a current
// update period that ends last.
func (c *Client) freeLicense(ctx context.Context, account string, owned []OwnedLicense) (string, error) {
	p, err := c.productInfo(ctx)
	if err != nil {
		return "", err
	}
	now := time.Now().UnixMilli()
	var best *OwnedLicense
	for i, l := range owned {
		if len(l.Devices) < p.MaxDevices && l.UpdatesUntilMs > now &&
			(best == nil || l.UpdatesUntilMs > best.UpdatesUntilMs) {
			best = &owned[i]
		}
	}
	if best == nil {
		return "", errors.New(account + " has no license of " + c.product.Key +
			" with a free seat; buy one, or ask the owner to remove a computer")
	}
	return best.TokenID, nil
}
