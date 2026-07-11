// Command rsx-maker is a dummy market maker for the RSX playground.
// It quotes both sides at a configurable spread around a reference
// price (mark when available, otherwise the BBO mid) and cancel-
// replaces on a fixed refresh interval. All configuration is via the
// environment; see config.go.
package main

import (
	"context"
	"log"
	"os"
	"os/signal"
	"syscall"
)

func main() {
	log.SetFlags(log.LstdFlags | log.Lmsgprefix)
	log.SetPrefix("rsx-maker ")

	cfg := loadConfig()
	if cfg.JWTSecret == "" {
		log.Fatal("RSX_GW_JWT_SECRET not set")
	}

	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer stop()

	bbo := newBBOSource()
	prices := newComposite(newMarkSource(), bbo)

	go runMarketdata(ctx, cfg, bbo)

	maker := newMaker(cfg, prices)
	maker.Run(ctx)

	log.Print("maker stopped")
	os.Exit(0)
}
