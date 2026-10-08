#!/usr/bin/env python3
"""Rebuild the local US/territory city index from attributed public GeoNames data."""
import datetime,hashlib,io,json,urllib.request,zipfile
from pathlib import Path
root=Path(__file__).resolve().parents[1]
base='https://download.geonames.org/export/dump/'
TERRITORIES={'AS':'American Samoa','GU':'Guam','MP':'Northern Mariana Islands','PR':'Puerto Rico','VI':'U.S. Virgin Islands'}
def fetch(name):
 with urllib.request.urlopen(base+name,timeout=90) as response:
  data=response.read(25_000_001)
 if len(data)>25_000_000:raise RuntimeError('Dataset exceeds expected download size')
 return data
archive=fetch('cities1000.zip');admin_bytes=fetch('admin1CodesASCII.txt')
admin={p[0]:p[1] for line in admin_bytes.decode().splitlines() if len(p:=line.split('\t'))>=2}
cities=[]
with zipfile.ZipFile(io.BytesIO(archive)) as z:
 info=z.getinfo('cities1000.txt')
 if info.file_size>100_000_000:raise RuntimeError('City dataset exceeds expected decompressed size')
 for line in z.read(info).decode().splitlines():
  p=line.split('\t')
  if p[8]!='US' and p[8] not in TERRITORIES:continue
  # GeoNames admin1 is the state for US rows but a municipality or district for territories.
  state,state_name=(p[10],admin.get('US.'+p[10],p[10])) if p[8]=='US' else (p[8],TERRITORIES[p[8]])
  cities.append(dict(id=int(p[0]),name=p[1],asciiName=p[2],state=state,stateName=state_name,country=p[8],latitude=float(p[4]),longitude=float(p[5]),population=int(p[14]),timeZone=p[17]))
if len(cities)<10000:raise RuntimeError('Unexpectedly small city index; preserving current data')
(root/'data/cities.json').write_text(json.dumps(cities,ensure_ascii=False,separators=(',',':'))+'\n')
(root/'data/SOURCE.md').write_text(f'''# City data

Derived from GeoNames cities1000.zip and admin1CodesASCII.txt, downloaded {datetime.date.today().isoformat()}. Filtered to the US and its territories (AS, GU, MP, PR, VI). Kept names, states, coordinates, population and time zone. For territories, `state` is the territory code and `stateName` the territory name, in place of the GeoNames municipality or district. Not a complete list of every settlement. City centers approximate a location, not an address.

Source: https://download.geonames.org/export/dump/
License: Creative Commons Attribution 4.0, https://creativecommons.org/licenses/by/4.0/
Attribution: GeoNames, https://www.geonames.org/
No endorsement implied. See scripts/update-cities.py to rebuild.

Input SHA-256: cities1000.zip `{hashlib.sha256(archive).hexdigest()}`; admin1CodesASCII.txt `{hashlib.sha256(admin_bytes).hexdigest()}`.
''')
print(f'Updated {len(cities)} cities. Review data diff before committing.')
