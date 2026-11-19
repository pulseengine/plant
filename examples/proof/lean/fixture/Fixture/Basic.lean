namespace Fixture

/-- Addition on naturals is commutative, by the library lemma. -/
theorem add_comm' (a b : Nat) : a + b = b + a := Nat.add_comm a b

/-- Doubling is adding a number to itself. -/
theorem double (n : Nat) : 2 * n = n + n := by omega

end Fixture
