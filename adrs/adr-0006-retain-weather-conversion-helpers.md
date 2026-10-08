# ADR-0006: Retain the weather conversion helpers

## Status

Accepted, 2026-10-01.

## Decision

Keep the current helpers for the present requirements. The module converts a
small, fixed set of NWS temperature and wind units, rounds display quantities,
preserves missing or unknown observations, converts plain forecast wind text,
and calculates spherical distance. Its tests cover those behaviors. A library
would not replace the NWS unit-code handling or the rules for missing values and
verbatim forecast text.

| Candidate | What it provides | When to reconsider |
| --- | --- | --- |
| [uom](https://docs.rs/uom/0.38.0/uom/) | Typed quantities, dimensional analysis, and unit conversion, including Celsius and Fahrenheit. | Calculations span more physical quantities and compile-time checks for mixed units become useful. |
| [geo](https://docs.rs/geo/latest/geo/) | Geometry operations and spherical Haversine or ellipsoidal geodesic distance. | A concrete feature needs geometry, route operations, or ellipsoidal distance accuracy. |
| Current helpers | The conversions and distance needed by this service, with no additional dependency. | Retain while the unit set and point-distance requirements remain small. |

This is a scope decision, not a claim that hand-written arithmetic is generally
better. If a library is adopted later, preserve unknown-unit behavior, nulls,
display rounding, and original forecast and alert instructions. Replacing only
distance must also preserve station ordering and the documented distance units.
