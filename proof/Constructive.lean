module
public import Lean
@[expose] public section

/-!
Constructive support for Webfold's finite, loop-free Aeneas extraction.

This is a deliberately small alternative extraction prelude, not the upstream
Aeneas interaction-tree runtime. Words are Lean BitVecs of the Rust width;
wrapping addition is modular, and ordinary arithmetic returns an explicit
overflow error. No runtime operation is postulated. Regeneration checks the
supported extraction surface and preserves every generated function body.
-/
namespace WebfoldModel

inductive Error where
  | assertionFailure | integerOverflow | divisionByZero | arrayOutOfBounds
  | maximumSizeExceeded | panic | undef

inductive Result (α : Type) where
  | ok (value : α)
  | fail (error : Error)

/-- Aeneas emits `lift` for a let-bound call to a pure (non-failing) function. -/
def lift {α : Type} (value : α) : Result α := .ok value

instance : Monad Result where
  pure := Result.ok
  bind value next := match value with
    | .ok x => next x
    | .fail e => .fail e

namespace Std

abbrev U8 := BitVec 8
abbrev U32 := BitVec 32
abbrev U64 := BitVec 64

def checkedAdd {width : Nat} (a b : BitVec width) : Result (BitVec width) :=
  if a.toNat + b.toNat < 2 ^ width then
    .ok (BitVec.ofNat width (a.toNat + b.toNat))
  else .fail .integerOverflow

def checkedSub {width : Nat} (a b : BitVec width) : Result (BitVec width) :=
  if b.toNat ≤ a.toNat then
    .ok (BitVec.ofNat width (a.toNat - b.toNat))
  else .fail .integerOverflow

instance {width : Nat} : HAdd (BitVec width) (BitVec width) (Result (BitVec width)) :=
  ⟨checkedAdd⟩

def checkedMul {width : Nat} (a b : BitVec width) : Result (BitVec width) :=
  if a.toNat * b.toNat < 2 ^ width then
    .ok (BitVec.ofNat width (a.toNat * b.toNat))
  else .fail .integerOverflow

/-- Rust `/` panics on a zero divisor; the quotient of in-range words is in range. -/
def checkedDiv {width : Nat} (a b : BitVec width) : Result (BitVec width) :=
  if b.toNat = 0 then .fail .divisionByZero
  else .ok (BitVec.ofNat width (a.toNat / b.toNat))

/-- Rust `%` panics on a zero divisor; the remainder is below the divisor. -/
def checkedRem {width : Nat} (a b : BitVec width) : Result (BitVec width) :=
  if b.toNat = 0 then .fail .divisionByZero
  else .ok (BitVec.ofNat width (a.toNat % b.toNat))

instance {width : Nat} : HSub (BitVec width) (BitVec width) (Result (BitVec width)) :=
  ⟨checkedSub⟩

instance {width : Nat} : HMul (BitVec width) (BitVec width) (Result (BitVec width)) :=
  ⟨checkedMul⟩

instance {width : Nat} : HDiv (BitVec width) (BitVec width) (Result (BitVec width)) :=
  ⟨checkedDiv⟩

instance {width : Nat} : HMod (BitVec width) (BitVec width) (Result (BitVec width)) :=
  ⟨checkedRem⟩

/-- The unsigned integer types the extraction names in `as` casts. -/
inductive UScalarTy where
  | U8 | U32 | U64

/-- The bit width of an unsigned integer type. -/
abbrev UScalarTy.bits : UScalarTy → Nat
  | .U8 => 8
  | .U32 => 32
  | .U64 => 64

/-- Rust `as` between unsigned integers: the value, truncated to the target width. -/
def UScalar.cast (ty : UScalarTy) {w : Nat} (x : BitVec w) : BitVec ty.bits :=
  BitVec.ofNat ty.bits x.toNat

def core.num.U32.wrapping_add (a b : U32) : U32 := BitVec.add a b

end Std
end WebfoldModel

macro:max n:num "#u8" : term => `(BitVec.ofNat 8 $n)
macro:max n:num "#u32" : term => `(BitVec.ofNat 32 $n)
macro:max n:num "#u64" : term => `(BitVec.ofNat 64 $n)
