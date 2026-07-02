// The Enum type behind "RFC Base"."eSeal Service" (fixture b, multi-level
// chain) — values are arbitrary, only the field's DECLARED TYPE (`Enum "RFC
// Service"`) matters for `classify_type_text` -> `ReceiverType::EnumType`.
enum 51501 "RFC Service"
{
    Extensible = true;

    value(0; ProviderA) { }
    value(1; ProviderB) { }
}
