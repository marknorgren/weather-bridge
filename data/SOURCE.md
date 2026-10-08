# City data

Derived from GeoNames cities1000.zip and admin1CodesASCII.txt, downloaded 2026-09-30. Filtered to the US and its territories (AS, GU, MP, PR, VI). Kept names, states, coordinates, population and time zone. For territories, `state` is the territory code and `stateName` the territory name, in place of the GeoNames municipality or district. Not a complete list of every settlement. City centers approximate a location, not an address.

Source: https://download.geonames.org/export/dump/
License: Creative Commons Attribution 4.0, https://creativecommons.org/licenses/by/4.0/
Attribution: GeoNames, https://www.geonames.org/
No endorsement implied. See scripts/update-cities.py to rebuild.

Input SHA-256: cities1000.zip `8cdadfcde41c82b9cc302bbaf8ec30132ef4fe2bb27ab0d6de5e5a4b701a15b5`; admin1CodesASCII.txt `1da92a6323a5fec3176f3f743bf4cf4040fd56a876da55e46fbca23c863aa60a`.
