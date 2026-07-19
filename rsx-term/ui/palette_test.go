package ui

import "testing"

// TestPaletteContrast is the reserved-contrast gate for the visual language:
// every ink the heatmap can put on a cell must clear the WCAG 1.4.11 3:1
// non-text floor on every background it can land on, in BOTH themes. The
// byte-locked view goldens strip ANSI, so colour regressions are invisible to
// them — this matrix is the test that sees colour. It would have caught the
// two historical fg==bg collapses: the "ramp one tier up" relief ink (1.00:1
// at the top tier) and the buy print on a tier-4 wall (Live on Live, 1.00:1).
func TestPaletteContrast(t *testing.T) {
	const minRatio = 3.0
	t.Cleanup(func() { UseTheme("") })
	for _, theme := range []string{"", "colorblind"} {
		UseTheme(theme)
		name := theme
		if name == "" {
			name = "default"
		}
		// Reserved glyph inks (density / persistence / micro) on every ramp tier.
		for tier := 0; tier <= sizeTiers; tier++ {
			bg := rampColor(tier)
			if r := contrastHex(string(glyphInk(bg)), string(bg)); r < minRatio {
				t.Errorf("%s: glyph ink on ramp tier %d = %.2f:1 < %.1f:1", name, tier, r, minRatio)
			}
		}
		// Overlay inks (trade aggressor hues, own-order accent) on every
		// background the overlay cap allows.
		for tier := 0; tier <= overlayBgTier; tier++ {
			bg := string(rampColor(tier))
			for ink, hex := range map[string]string{
				"buy (Live)":   string(ColorLive),
				"sell (Ask)":   string(ColorAsk),
				"own (Accent)": string(ColorAccent),
			} {
				if r := contrastHex(hex, bg); r < minRatio {
					t.Errorf("%s: %s on overlay tier %d = %.2f:1 < %.1f:1", name, ink, tier, r, minRatio)
				}
			}
		}
		// Text inks on the page background.
		for ink, hex := range map[string]string{
			"Text":       string(ColorText),
			"TextBright": string(ColorTextBright),
			"Live":       string(ColorLive),
			"Ask":        string(ColorAsk),
			"Heading":    string(ColorHeading),
			"Accent":     string(ColorAccent),
			"Degraded":   string(ColorDegraded),
			"Muted":      string(ColorMuted),
		} {
			if r := contrastHex(hex, string(ColorPageBg)); r < minRatio {
				t.Errorf("%s: %s on page = %.2f:1 < %.1f:1", name, ink, r, minRatio)
			}
		}
	}
}
