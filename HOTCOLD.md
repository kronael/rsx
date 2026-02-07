# Hot/Cold Field Splitting

## Concept

Hot/cold field splitting is a data layout optimization that separates frequently accessed ("hot") fields from rarely accessed ("cold") fields into distinct structs or memory regions. This improves cache utilization by packing hot data together so cache lines aren't wasted on cold fields.

## The Problem

Consider a struct where only a few fields are accessed in a tight loop:

```rust
struct Entity {
    // Hot — accessed every frame
    position: Vec3,
    velocity: Vec3,

    // Cold — accessed rarely (editor, serialization, debug)
    name: String,
    created_at: Instant,
    metadata: HashMap<String, String>,
}
```

A `Vec<Entity>` interleaves hot and cold data in memory. Every cache line pull brings in `name`, `created_at`, and `metadata` even though the inner loop only touches `position` and `velocity`. This wastes cache capacity and bandwidth.

## The Solution

Split into two structs and store them in parallel containers:

```rust
struct EntityHot {
    position: Vec3,
    velocity: Vec3,
}

struct EntityCold {
    name: String,
    created_at: Instant,
    metadata: HashMap<String, String>,
}

struct World {
    hot: Vec<EntityHot>,
    cold: Vec<EntityCold>,
}
```

Now iterating over `world.hot` touches only the data the physics loop needs. Cache lines are fully utilized.

## When to Apply

| Situation | Benefit |
|---|---|
| Tight loops touching a few fields of a large struct | High — fewer cache misses |
| Mixed read/write patterns (some fields read-only, others mutated) | Medium — avoids false sharing |
| SIMD-friendly layouts (SoA) | High — enables vectorized loads |
| Small structs where all fields are used together | None — splitting adds indirection |

## Struct-of-Arrays (SoA) — The Extreme Case

Hot/cold splitting taken to its logical extreme is **Struct-of-Arrays (SoA)**:

```rust
// AoS (Array of Structs) — default Rust layout
struct Particles {
    data: Vec<Particle>, // [pos, vel, color, pos, vel, color, ...]
}

// SoA (Struct of Arrays) — each field in its own contiguous array
struct Particles {
    positions:  Vec<Vec3>,
    velocities: Vec<Vec3>,
    colors:     Vec<Color>,
}
```

SoA is especially powerful when combined with SIMD, since each array is a contiguous lane of identical types.

## Relationship to ECS

Entity Component System (ECS) architectures (e.g., `bevy_ecs`, `hecs`, `legion`) are essentially automatic hot/cold splitting at the archetype level. Components are stored in dense, type-homogeneous arrays, and queries pull only the component arrays they need.

## Trade-offs

- **Pros:** Better cache utilization, enables SIMD, reduces false sharing in concurrent code.
- **Cons:** Adds complexity, requires parallel indexing, can hurt readability if overused.
- **Rule of thumb:** Profile first. Split only when cache misses are a measured bottleneck.
