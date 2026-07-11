package ui

import (
	"os"
	"strings"
	"testing"
)

// TestGlyphVocabularyDocumented asserts every glyph the renderer can emit is
// in VISUALS.md's legend — the vocabulary and its documentation must never
// drift apart.
func TestGlyphVocabularyDocumented(t *testing.T) {
	doc, err := os.ReadFile("../VISUALS.md")
	if err != nil {
		t.Fatalf("read VISUALS.md: %v", err)
	}
	legend := string(doc)
	all := string(glyphs.countRamp) + string(glyphs.tradeRamp) +
		string(glyphs.microRamp) + string(glyphs.newsRamp) +
		string([]rune{
			glyphs.persistent, glyphs.ownOrder, glyphs.ownBuy, glyphs.ownSell,
			glyphs.cursor, glyphs.touchTick, glyphs.rulerLine, glyphs.railIdle,
		})
	for _, r := range all {
		if r == ' ' {
			continue
		}
		if !strings.ContainsRune(legend, r) {
			t.Fatalf("glyph %q is renderable but undocumented in VISUALS.md", r)
		}
	}
}

// TestGlyphVocabularyExcludesBraille guards the calibration finding: braille
// renders as tofu in DejaVuSansMono, so no table entry may use it.
func TestGlyphVocabularyExcludesBraille(t *testing.T) {
	all := string(glyphs.countRamp) + string(glyphs.tradeRamp) +
		string(glyphs.microRamp) + string(glyphs.newsRamp)
	for _, r := range all {
		if r >= 0x2800 && r <= 0x28FF {
			t.Fatalf("braille glyph %q in the vocabulary (tofu in DejaVuSansMono)", r)
		}
	}
}
