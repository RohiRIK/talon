# ADR 0003 — No async-trait Crate

**Status:** Accepted  
**Date:** 2026-05-27

## Context

Before Rust Edition 2024, `async fn` in trait definitions was not stable. The `async-trait` proc-macro worked around this by desugaring `async fn` into `fn(...) -> Pin<Box<dyn Future + Send>>`. Edition 2024 stabilized Return Position Impl Trait (RPIT) in traits, making `async fn` in trait definitions work natively.

## Decision

`async-trait` is banned in `deny.toml`. All traits with `async fn` methods use edition 2024 native syntax:

```rust
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, messages: &[Message]) -> Result<LlmResponse, LlmError>;
}
```

## Object Safety Caveat

`async fn` in traits is NOT object-safe. `dyn LlmProvider` will not compile because the compiler cannot know the concrete `Future` type at the call site.

**If `dyn LlmProvider` is needed** (e.g., for runtime-switchable providers):
- Option A: Return `Pin<Box<dyn Future<Output = Result<...>> + Send + '_>>` from the trait method
- Option B: Concrete enum dispatch (`enum AnyProvider { Anthropic(AnthropicProvider), OpenAI(OpenAiProvider) }`)

For Phase 1, we start with concrete generics (`impl LlmProvider`) and defer the object-safety decision until the need is confirmed.

## Consequences

- Simpler trait definitions
- No proc-macro compilation overhead
- Object safety must be handled explicitly if `dyn Trait` is needed
