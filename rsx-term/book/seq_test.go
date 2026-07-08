package book

import "testing"

func TestSeqFirstObserveNeverGap(t *testing.T) {
	var tr SeqTracker
	if gap := tr.Observe(500); gap {
		t.Fatalf("first Observe() reported a gap")
	}
}

func TestSeqNextIsNotGap(t *testing.T) {
	var tr SeqTracker
	tr.Observe(1)
	if gap := tr.Observe(2); gap {
		t.Fatalf("Observe(last+1) reported a gap")
	}
}

func TestSeqDuplicateIsNotGap(t *testing.T) {
	var tr SeqTracker
	tr.Observe(1)
	if gap := tr.Observe(1); gap {
		t.Fatalf("Observe(same seq) reported a gap")
	}
	if gap := tr.Observe(1); gap {
		t.Fatalf("Observe(same seq) repeated reported a gap")
	}
}

func TestSeqJumpIsGap(t *testing.T) {
	var tr SeqTracker
	tr.Observe(1)
	if gap := tr.Observe(3); !gap {
		t.Fatalf("Observe(last+2) did not report a gap")
	}
}

func TestSeqLowerAfterGapIsNotGap(t *testing.T) {
	var tr SeqTracker
	tr.Observe(1)
	tr.Observe(10) // gap, last becomes 10
	if gap := tr.Observe(5); gap {
		t.Fatalf("Observe(lower than last) after a gap reported a gap")
	}
}

func TestSeqResetTo(t *testing.T) {
	var tr SeqTracker
	tr.Observe(1)
	tr.Observe(10)
	tr.ResetTo(50)
	if gap := tr.Observe(51); gap {
		t.Fatalf("Observe(seq+1) after ResetTo(seq) reported a gap")
	}
}
