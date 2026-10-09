#!/usr/bin/env python3
"""Terraform deployment entry point. Enable only after the usage guard succeeds."""
import argparse
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import subprocess
import sys
from urllib.parse import urlparse

ROOT = Path(__file__).resolve().parents[2]
MODULE = ROOT / 'infra/aws/terraform'
sys.path.insert(0, str(ROOT / 'scripts'))
from release import ReleaseError, artifact_path, verify_release  # noqa: E402


def aws_option(value, label):
    message = f'Invalid AWS {label}: use a name without option prefixes or controls'
    if not 1 <= len(value) <= 128 or value[0] not in 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789':
        raise argparse.ArgumentTypeError(message)
    # Construct each option from checked ASCII characters, with no raw input left
    # in the subprocess argument array.
    characters = []
    for character in value:
        if character not in (
            'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M',
            'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z',
            'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm',
            'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z',
            '0', '1', '2', '3', '4', '5', '6', '7', '8', '9',
            '_', '.', '@', ':', '/', ' ', '-',
        ):
            raise argparse.ArgumentTypeError(message)
        characters.append(character)
    return ''.join(characters)


def aws_profile(value):
    return aws_option(value, 'profile')


def aws_region(value):
    value = aws_option(value, 'region')
    if not re.fullmatch(r'[a-z]{2,4}(?:-[a-z]{1,16}){1,3}-[0-9]{1,2}', value):
        raise argparse.ArgumentTypeError('Invalid AWS region: use a region name such as us-east-1')
    return value

parser = argparse.ArgumentParser()
parser.add_argument('--profile', type=aws_profile,
                    help='AWS CLI profile; omit to use the standard credential chain (for example OIDC)')
parser.add_argument('--region', default='us-east-1', type=aws_region)
parser.add_argument('--contact', required=True, help='Public NWS contact URL or email')
parser.add_argument('--release', required=True, type=artifact_path,
                    help='Directory containing app.zip, guard.zip, and release-manifest.json')
parser.add_argument('--revision', required=True,
                    help='Exact 40-character Git revision expected in the verified release')
parser.add_argument('--concurrency', type=int, default=3, choices=range(1, 11), metavar='1-10',
                    help='Reserved concurrency restored by the guard (default 3)')
parser.add_argument('--cloudfront-plan', choices=['PAY_AS_YOU_GO', 'FREE'],
                    help='CloudFront pricing: preserve the existing plan (PAY_AS_YOU_GO for a new demo), or the $0 '
                         'flat-rate Free plan. Check eligibility first; see infra/aws/README.md.')
parser.add_argument('--domain', help='Optional custom hostname; omitted preserves the existing hostname')
parser.add_argument('--prepare-domain', action='store_true',
                    help='Request the Terraform-managed certificate and print DNS records without pausing the demo')
args = parser.parse_args()

# Release verification must finish before any AWS or Terraform command. The copied files are
# verified again so every later step uses exactly the bytes that passed validation.
try:
    release_manifest = verify_release(args.release, args.revision)
except ReleaseError as error:
    raise SystemExit(f'Release verification failed before AWS access: {error}') from error
try:
    target_dir = artifact_path(ROOT / 'target', root=ROOT)
    state_dir = artifact_path(target_dir / 'aws-deployment', root=target_dir)
    state_dir.mkdir(parents=True, exist_ok=True)
    copies = [(artifact_path(args.release / name, root=args.release),
               artifact_path(state_dir / name, root=state_dir))
              for name in ('app.zip', 'guard.zip', 'release-manifest.json')]
    for source, destination in copies:
        shutil.copyfile(source, destination)
    release_manifest = verify_release(state_dir, args.revision)
except ReleaseError as error:
    raise SystemExit(f'Staged release verification failed before AWS access: {error}') from error


def aws(*parts, json_output=False, allow_missing=False):
    command = ['aws', *parts]
    if args.profile:
        command.append('--profile=' + aws_profile(args.profile))
    command.extend(('--region=' + aws_region(args.region), '--no-cli-pager'))
    result = subprocess.run(command, text=True, capture_output=json_output or allow_missing)
    if result.returncode:
        if allow_missing and 'does not exist' in result.stderr:
            return None
        raise RuntimeError(result.stderr or 'AWS CLI command failed')
    return json.loads(result.stdout) if json_output else None


def tf(*parts, capture=False, check=True):
    result = subprocess.run(['terraform', '-chdir=' + str(MODULE), *parts],
                            text=True, capture_output=capture, check=check)
    return result


def invoke(action):
    path = state_dir / ('guard-' + action + '.json')
    result = aws('lambda', 'invoke', '--function-name', 'weather-bridge-usage-guard',
                 '--cli-binary-format', 'raw-in-base64-out', '--payload', json.dumps({'action': action}),
                 str(path), json_output=True)
    payload = json.loads(path.read_text())
    print(json.dumps(payload, indent=2), flush=True)
    if result.get('FunctionError'):
        raise RuntimeError('Usage guard failed; the demo remains stopped. See its CloudWatch logs.')
    return payload


existing = {}
for name in ['weather-bridge-artifacts', 'weather-bridge-demo']:
    response = aws('cloudformation', 'describe-stacks', '--stack-name', name,
                   json_output=True, allow_missing=True)
    if response:
        existing[name] = response['Stacks'][0]


if 'weather-bridge-demo' in existing:
    required_outputs = {'DistributionId', 'Endpoint', 'FunctionUrl'}
    output_names = {item['OutputKey'] for item in existing['weather-bridge-demo'].get('Outputs', [])}
    if not required_outputs.issubset(output_names):
        raise SystemExit('Deployment requires a CloudFront-based stack; legacy Function URL stacks are no longer supported.')


def origin_verify_secret():
    # Kept in ignored local state so repeated deploys keep the same value. It is a bypass
    # deterrent for the Function URL, not a credential; see infra/aws/README.md.
    path = state_dir / 'origin-verify-secret'
    if not path.is_file():
        # Create it owner-only so it is never readable by other users, even briefly.
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, 'w') as f:
            f.write(secrets.token_urlsafe(48) + '\n')
    path.chmod(0o600)
    return path.read_text().strip()


def public_origin(url):
    return 'https://' + urlparse(url).hostname + ':443'


parameters = {'profile': args.profile, 'region': args.region, 'nws_contact': args.contact, 'public_origin': '',
              'origin_host': '', 'origin_verify_secret': origin_verify_secret(),
              'reserved_concurrency': args.concurrency, 'cloudfront_pricing_plan': args.cloudfront_plan or 'PAY_AS_YOU_GO'}
if 'weather-bridge-demo' in existing:
    outputs = {o['OutputKey']: o['OutputValue'] for o in existing['weather-bridge-demo'].get('Outputs', [])}
    parameters['public_origin'] = public_origin(outputs['Endpoint'])
    parameters['origin_host'] = urlparse(outputs['FunctionUrl']).hostname
    previous_domain = next((p['ParameterValue'] for p in existing['weather-bridge-demo'].get('Parameters', [])
                            if p['ParameterKey'] == 'CustomDomain'), '')
    if args.cloudfront_plan is None:
        parameters['cloudfront_pricing_plan'] = next((p['ParameterValue'] for p in existing['weather-bridge-demo'].get('Parameters', [])
                                                     if p['ParameterKey'] == 'CloudFrontPricingPlan'), 'PAY_AS_YOU_GO')
else:
    previous_domain = ''
parameters['custom_domain'] = previous_domain if args.domain is None else args.domain.strip().lower()
if previous_domain and parameters['custom_domain'] and parameters['custom_domain'] != previous_domain:
    raise SystemExit('Changing an existing hostname requires a separate certificate migration; the demo was not changed.')
if args.prepare_domain and not parameters['custom_domain']:
    raise SystemExit('--prepare-domain requires --domain or an existing custom hostname')


def write_settings():
    # This local file is ignored. Public examples contain placeholders only.
    (MODULE / 'deployment.auto.tfvars').write_text(''.join(k + ' = ' + json.dumps(v) + '\n' for k, v in parameters.items()))
    tf('fmt', 'deployment.auto.tfvars')


write_settings()
tf('init', '-input=false')
for name, address in [('weather-bridge-artifacts', 'aws_cloudformation_stack.artifacts'),
                      ('weather-bridge-demo', 'aws_cloudformation_stack.demo')]:
    if name in existing and tf('state', 'show', address, capture=True, check=False).returncode:
        tf('import', '-input=false', address, existing[name]['StackId'])

# GitHub-hosted runners start without local Terraform state. Import the objects selected by
# the running stack so create_before_destroy can retain them through the update and delete
# them only after CloudFormation has switched to the verified release.
if 'weather-bridge-artifacts' in existing and 'weather-bridge-demo' in existing:
    artifact_outputs = {item['OutputKey']: item['OutputValue']
                        for item in existing['weather-bridge-artifacts'].get('Outputs', [])}
    deployed_parameters = {item['ParameterKey']: item.get('ParameterValue', '')
                           for item in existing['weather-bridge-demo'].get('Parameters', [])}
    bucket = artifact_outputs.get('Bucket')
    for key_name, address in [('AppKey', 'aws_s3_object.app'), ('GuardKey', 'aws_s3_object.guard')]:
        key = deployed_parameters.get(key_name)
        if bucket and key and tf('state', 'show', address, capture=True, check=False).returncode:
            tf('import', '-input=false', address, bucket + '/' + key)
    certificate = deployed_parameters.get('CertificateArn')
    certificate_address = 'aws_acm_certificate.public[0]'
    if (certificate and tf('state', 'show', certificate_address, capture=True, check=False).returncode):
        tf('import', '-input=false', certificate_address, certificate)

def apply(*plan_args):
    plan = state_dir / 'deployment.tfplan'
    tf('plan', '-input=false', '-out=' + str(plan), *plan_args)
    # Inspect the plan before applying: this module must only manage these own resources.
    contents = json.loads(tf('show', '-json', str(plan), capture=True).stdout)
    allowed = {'aws_cloudformation_stack.artifacts', 'aws_cloudformation_stack.demo', 'aws_s3_object.app', 'aws_s3_object.guard',
               'aws_acm_certificate.public[0]'}
    if any(c['address'] not in allowed for c in contents.get('resource_changes', [])):
        raise RuntimeError('Unexpected resource in deployment plan; not applying.')
    tf('apply', '-input=false', str(plan))


def output(name):
    return json.loads(tf('output', '-json', name, capture=True).stdout)


if parameters['custom_domain']:
    # Certificate preparation is separate so external DNS validation never pauses a live demo.
    apply('-target=aws_acm_certificate.public')
    certificate = aws('acm', 'describe-certificate', '--certificate-arn', output('certificate_arn'),
                      json_output=True)['Certificate']
    records = [v['ResourceRecord'] for v in certificate.get('DomainValidationOptions', []) if 'ResourceRecord' in v]
    print(json.dumps({'domain': parameters['custom_domain'], 'certificateStatus': certificate['Status'],
                      'validationRecords': records}, indent=2), flush=True)
    if args.prepare_domain:
        raise SystemExit(0)
    if certificate['Status'] != 'ISSUED':
        raise SystemExit('Add the validation CNAME to your DNS provider, then re-run this command. The demo was not paused.')
    parameters['public_origin'] = public_origin('https://' + parameters['custom_domain'])
    write_settings()

if 'weather-bridge-demo' in existing:
    aws('lambda', 'put-function-concurrency', '--function-name', 'weather-bridge-demo',
        '--reserved-concurrent-executions', '0')
apply()
endpoint = output('endpoint')
discovered = {'public_origin': public_origin(endpoint), 'origin_host': urlparse(output('function_url')).hostname}
if any(parameters[key] != value for key, value in discovered.items()):
    # The app's MCP Host/Origin allowlist needs the generated CloudFront and Function URL hosts.
    parameters.update(discovered)
    write_settings()
    apply()
distribution = output('distribution_id')
# Cached pages and assets must not outlive this deploy. One wildcard path counts as one of
# the 1,000 free invalidation paths per month.
aws('cloudfront', 'create-invalidation', '--distribution-id', distribution, '--paths', '/*')
state = {'profile': args.profile, 'region': args.region, 'endpoint': endpoint,
         'distributionEndpoint': output('distribution_endpoint'), 'customDomain': parameters['custom_domain'],
         'functionUrlHost': parameters['origin_host'], 'distributionId': distribution,
         'releaseRevision': release_manifest['revision'],
         'appArtifactSha256': release_manifest['artifacts']['app.zip']['sha256'],
         'guardArtifactSha256': release_manifest['artifacts']['guard.zip']['sha256'],
         'stack': 'weather-bridge-demo', 'artifactStack': 'weather-bridge-artifacts',
         'demoFunction': 'weather-bridge-demo', 'guardFunction': 'weather-bridge-usage-guard'}
(state_dir / 'state.json').write_text(json.dumps(state, indent=2) + '\n')
if invoke('resume')['stopped']:
    raise RuntimeError('The account exceeds configured usage thresholds. The demo remains stopped.')
print('Weather Bridge endpoint: ' + endpoint, flush=True)
