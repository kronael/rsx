/**
 * Fixed-size ring buffer. Overwrites oldest entry when full.
 * Useful for caching recent trades, ticks, or price history.
 */
export class RingBuffer<T> {
  private buf: (T | undefined)[];
  private head = 0; // next write position
  private count = 0;
  readonly capacity: number;

  constructor(capacity: number) {
    if (capacity < 1) throw new RangeError("capacity must be >= 1");
    this.capacity = capacity;
    this.buf = new Array(capacity);
  }

  push(value: T): void {
    this.buf[this.head] = value;
    this.head = (this.head + 1) % this.capacity;
    if (this.count < this.capacity) this.count++;
  }

  /** Most recent item, or undefined if empty. */
  last(): T | undefined {
    if (this.count === 0) return undefined;
    const idx = (this.head - 1 + this.capacity) % this.capacity;
    return this.buf[idx];
  }

  /** Oldest item still in the buffer, or undefined if empty. */
  first(): T | undefined {
    if (this.count === 0) return undefined;
    const idx =
      this.count < this.capacity
        ? 0
        : this.head % this.capacity;
    return this.buf[idx];
  }

  get size(): number {
    return this.count;
  }

  get isFull(): boolean {
    return this.count === this.capacity;
  }

  get isEmpty(): boolean {
    return this.count === 0;
  }

  /**
   * Returns items oldest-first as a plain array.
   * Allocates — avoid on hot path.
   */
  toArray(): T[] {
    if (this.count === 0) return [];
    const out: T[] = new Array(this.count);
    const start =
      this.count < this.capacity ? 0 : this.head;
    for (let i = 0; i < this.count; i++) {
      out[i] = this.buf[(start + i) % this.capacity] as T;
    }
    return out;
  }

  /**
   * Iterate oldest-first without allocation.
   */
  forEach(fn: (value: T, index: number) => void): void {
    const start =
      this.count < this.capacity ? 0 : this.head;
    for (let i = 0; i < this.count; i++) {
      fn(this.buf[(start + i) % this.capacity] as T, i);
    }
  }

  /** Remove all items. */
  clear(): void {
    this.buf = new Array(this.capacity);
    this.head = 0;
    this.count = 0;
  }
}
