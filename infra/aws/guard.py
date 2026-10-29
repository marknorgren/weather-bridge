"""Account-wide usage guard; only the named demo function can be stopped."""
import concurrent.futures
import datetime as dt
import json
import os
from decimal import Decimal

TARGET = os.environ["TARGET_FUNCTION"]
REGION = os.environ["TARGET_REGION"]
REQUEST_LIMIT = float(os.environ.get("REQUEST_LIMIT", "800000"))
COMPUTE_LIMIT = float(os.environ.get("COMPUTE_LIMIT_GB_SECONDS", "320000"))
# CloudFront: 80% of the always-free 10,000,000 requests and 1 TB transfer per month,
# summed over every distribution in the account. Its control plane and metrics are in us-east-1.
DISTRIBUTION_ID = os.environ.get("DISTRIBUTION_ID", "")
CLOUDFRONT_REQUEST_LIMIT = float(os.environ.get("CLOUDFRONT_REQUEST_LIMIT", "8000000"))
CLOUDFRONT_BYTES_LIMIT = float(os.environ.get("CLOUDFRONT_BYTES_LIMIT", "800000000000"))
CLOUDFRONT_REGION = "us-east-1"
MAX_RESUME_CONCURRENCY = 10
# Each cold start is charged the on-demand Init phase limit (10 s). Duration excludes Init;
# a suppressed re-init after an invoke failure is already inside Duration.
INIT_CAP_SECONDS = 10.0
# The demo's execution-environment starts are counted by a CloudWatch Logs metric filter on
# Lambda's INIT_START system log line (platform.initStart in JSON log format), which Lambda
# writes each time it creates an execution environment. Other functions have no such
# evidence, so every one of their invocations is charged a cold start, as before.
INIT_FILTER_NAME = os.environ.get("INIT_FILTER_NAME", "weather-bridge-demo-init-starts")
INIT_FILTER_PATTERN = '?"INIT_START" ?"platform.initStart"'
INIT_METRIC_NAMESPACE = os.environ.get("INIT_METRIC_NAMESPACE", "WeatherBridge")
INIT_METRIC_NAME = os.environ.get("INIT_METRIC_NAME", "DemoInitStarts")
# Log delivery and metric publication can trail Invocations by minutes. Trust only complete
# hours before this window; conservatively charge every newer invocation a cold start.
LOG_DELIVERY_LAG = dt.timedelta(minutes=15)
STATE_BUCKET = os.environ.get("STATE_BUCKET", "")
STATE_KEY = "guard-state/state.json"
STOP_TOPIC_ARN = os.environ.get("STOP_TOPIC_ARN", "")
OWNER_TOPIC_ARN = os.environ.get("OWNER_TOPIC_ARN", "")
SCHEDULE_ARN = os.environ.get("SCHEDULE_ARN", "")
ACCOUNT_ID = os.environ.get("ACCOUNT_ID", "")
PARTITION = os.environ.get("PARTITION", "aws")
CONTROL_ALARMS = ("weather-bridge-guard-error", "weather-bridge-schedule-failure")
BUDGET_NAME = "weather-bridge-lambda-backstop"
CHECK_INTERVAL = dt.timedelta(minutes=5)
TEMPORARY_CODES = frozenset({"InternalFailure", "InternalError", "ServiceException",
                            "ServiceUnavailable", "ServiceUnavailableException", "Throttling",
                            "ThrottlingException", "TooManyRequestsException", "RequestLimitExceeded",
                            "RequestTimeout", "RequestTimeoutException", "InternalServiceFault",
                            "InternalErrorException", "Unavailable", "SlowDown"})


class UsageReadError(Exception):
    """Only failures at a usage-read boundary can create a recoverable pause."""
    def __init__(self, service, region, operation, error):
        response = getattr(error, "response", {})
        self.details = {"service": service, "region": region, "operation": operation,
                        "code": response.get("Error", {}).get("Code", type(error).__name__),
                        "awsRequestId": response.get("ResponseMetadata", {}).get("RequestId")}
        status = response.get("ResponseMetadata", {}).get("HTTPStatusCode")
        server_failure = type(status) is int and (status == 429 or 500 <= status <= 599)
        self.temporary = (server_failure or self.details["code"] in TEMPORARY_CODES or type(error).__name__ in {
            "EndpointConnectionError", "ConnectTimeoutError", "ReadTimeoutError", "ConnectionClosedError"})
        super().__init__("usage read failed")


def usage_read(sdk, operation, **arguments):
    try:
        return getattr(sdk, operation)(**arguments)
    except Exception as error:
        raise UsageReadError(sdk.meta.service_model.service_name, sdk.meta.region_name,
                             operation, error) from None


def usage_pages(sdk, operation):
    try:
        yield from sdk.get_paginator(operation).paginate()
    except Exception as error:
        raise UsageReadError(sdk.meta.service_model.service_name, sdk.meta.region_name,
                             operation, error) from None


def utcnow():
    return dt.datetime.now(dt.timezone.utc)


def timestamp(value):
    moment = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    if moment.utcoffset() != dt.timedelta(0):
        raise ValueError("guard timestamps must be UTC")
    return moment


class StateError(ValueError):
    pass


class MissingState(StateError):
    """The authorized prefix listing proved that the record does not exist."""


def state_record(mode, reason, previous=None):
    return {"version": 1, "mode": mode, "reason": reason, "since": utcnow().isoformat(),
            "successes": 0, "lastCheck": None, "signals": (previous or {}).get("signals", [])}


def valid_deployment_id(value):
    return isinstance(value, str) and len(value) == 32 and all(c in "0123456789abcdef" for c in value)


def deployment_matches(state, identifier):
    return ((state["mode"], state["reason"]) == ("locked", "deployment_stop")
            and valid_deployment_id(identifier) and state.get("deploymentId") == identifier)


def validate_state(state):
    try:
        if (type(state["version"]) is not int or state["version"] != 1 or state["mode"] not in {"running", "paused", "locked"}
                or not isinstance(state["reason"], str) or not 1 <= len(state["reason"]) <= 64
                or type(state["successes"]) is not int or state["successes"] not in (0, 1)
                or not isinstance(state["signals"], list) or len(state["signals"]) > 16
                or any(not isinstance(s, str) or not 1 <= len(s) <= 128 for s in state["signals"])):
            raise ValueError("invalid state fields")
        timestamp(state["since"])
        if state["lastCheck"] is not None:
            if timestamp(state["lastCheck"]) <= timestamp(state["since"]):
                raise ValueError("recovery evidence predates the pause")
        if (state["successes"] == 1) != (state["lastCheck"] is not None):
            raise ValueError("invalid recovery evidence")
        if state["mode"] != "paused" and state["successes"]:
            raise ValueError("only a pause can have recovery evidence")
        if state["mode"] == "paused" and state["reason"] != "monitoring_failure":
            raise ValueError("unknown recovery eligibility")
        if "deploymentId" in state and (not valid_deployment_id(state["deploymentId"])
                or (state["mode"], state["reason"]) != ("locked", "deployment_stop")):
            raise ValueError("invalid deployment ownership")
    except (KeyError, TypeError, ValueError, AttributeError) as error:
        raise StateError("missing or malformed guard state; explicit resume required") from error
    return state


def load_state():
    s3 = client("s3", REGION)
    # A prefix-limited list distinguishes missing state from denied reads. S3
    # can return 403 for a missing object when the caller cannot list the bucket.
    listing = s3.list_objects_v2(Bucket=STATE_BUCKET, Prefix=STATE_KEY, MaxKeys=1)
    if not any(item["Key"] == STATE_KEY for item in listing.get("Contents", [])):
        raise MissingState("guard state is missing; explicit resume required")
    response = s3.get_object(Bucket=STATE_BUCKET, Key=STATE_KEY)
    body = response["Body"]
    try:
        value = body.read(4097)
    finally:
        body.close()
    try:
        if len(value) > 4096:
            raise StateError("guard state exceeds its size limit")
        return validate_state(json.loads(value))
    except (ValueError, TypeError) as error:
        raise StateError("guard state is malformed; explicit resume required") from error


def save_state(state):
    validate_state(state)
    value = json.dumps(state).encode()
    if len(value) > 4096:
        raise StateError("guard state exceeds its size limit")
    client("s3", REGION).put_object(Bucket=STATE_BUCKET, Key=STATE_KEY, Body=value,
                                   ServerSideEncryption="AES256", ContentType="application/json")


def notify(event, state):
    message = {"event": event, "mode": state["mode"], "reason": state["reason"], "since": state["since"]}
    print(json.dumps(message))
    if OWNER_TOPIC_ARN:
        try:
            client("sns", REGION).publish(TopicArn=OWNER_TOPIC_ARN,
                                          Subject="Weather Bridge: " + event, Message=json.dumps(message))
        except Exception as error:
            # A notification outage must not change cost-control state or undo a safe resume.
            print(json.dumps({"event": "notification_failed", "errorType": type(error).__name__}))


def stop_with_state(state, mode, reason, signals=None, deployment_id=None):
    updated = state_record(mode, reason, state)
    if deployment_id is not None:
        updated["deploymentId"] = deployment_id
    if signals is not None:
        updated["signals"] = signals[-16:]
    save_state(updated)  # Persist ownership before changing either service.
    stop_demo()
    if (state["mode"], state["reason"]) != (mode, reason):
        notify("paused" if mode == "paused" else "stopped", updated)
    return updated


def recovery_blocker():
    cw = client("cloudwatch", REGION)
    alarms = usage_read(cw, "describe_alarms", AlarmNames=list(CONTROL_ALARMS))["MetricAlarms"]
    if ({a["AlarmName"] for a in alarms} != set(CONTROL_ALARMS)
            or any(a["StateValue"] != "OK" for a in alarms)):
        return "control_alarm"
    budget = usage_read(client("budgets", REGION), "describe_budget", AccountId=ACCOUNT_ID,
                        BudgetName=BUDGET_NAME)["Budget"]
    if "CalculatedSpend" not in budget:
        return "budget_unknown"
    actual, limit = budget["CalculatedSpend"]["ActualSpend"], budget["BudgetLimit"]
    spent, amount = Decimal(actual["Amount"]), Decimal(limit["Amount"])
    if (actual["Unit"] != "USD" or limit["Unit"] != "USD" or not spent.is_finite()
            or not amount.is_finite() or spent < 0 or amount <= 0):
        raise ValueError("invalid budget evidence")
    # Matches template.json: actual charges GREATER_THAN 100% of the USD 1 budget.
    return "budget_limit" if spent > amount else None


def signal_reason(event, state):
    records = event.get("Records", [])
    if not records:
        return None, state["signals"]
    signals = list(state["signals"])
    reason = None
    for record in records:
        sns = record.get("Sns", {})
        identifier = sns.get("MessageId")
        if (record.get("EventSource") != "aws:sns" or not STOP_TOPIC_ARN
                or sns.get("TopicArn") != STOP_TOPIC_ARN or not isinstance(identifier, str)
                or not 1 <= len(identifier) <= 128):
            raise ValueError("unrecognized stop signal")
        if identifier in signals:
            continue
        signals.append(identifier)
        # Only this topic, whose policy restricts publishers, can request a signal stop.
        # Unknown payloads stay locked. Never infer temporary-read recovery from SNS text.
        reason = "budget_signal"
        try:
            message = json.loads(sns["Message"])
        except (ValueError, TypeError, KeyError):
            message = None
        if isinstance(message, dict):
            expected = {"arn:" + PARTITION + ":cloudwatch:" + REGION + ":" + ACCOUNT_ID + ":alarm:" + name
                        for name in CONTROL_ALARMS}
            reason = "control_alarm" if message.get("AlarmArn") in expected else "unknown_signal"
    return reason, signals[-16:]


def recovery_check_time(event, state):
    if (not SCHEDULE_ARN or event.get("source") != "aws.events"
            or event.get("detail-type") != "Scheduled Event"
            or event.get("resources") != [SCHEDULE_ARN]
            or not isinstance(event.get("id"), str) or not event["id"]):
        return None
    try:
        moment = timestamp(event["time"])
    except (KeyError, TypeError, ValueError, AttributeError):
        return None
    if not dt.timedelta(0) <= utcnow() - moment <= CHECK_INTERVAL:
        return None
    if moment <= timestamp(state["since"]):
        return None
    if state["lastCheck"] and moment - timestamp(state["lastCheck"]) < CHECK_INTERVAL:
        return None
    return moment.isoformat()


def resume_demo(state, reason):
    pending = state_record("locked", "resume_in_progress", state)
    save_state(pending)  # A crash after enabling cannot leave automatic-recovery authority.
    set_concurrency(RESUME_CONCURRENCY)
    set_distribution_enabled(True)
    if not distribution_enabled() or current_concurrency() != RESUME_CONCURRENCY:
        raise RuntimeError("resume verification failed")
    running = state_record("running", reason, pending)
    save_state(running)
    notify("recovered", running)
    return running


def current_concurrency():
    return client("lambda", REGION).get_function_concurrency(FunctionName=TARGET).get("ReservedConcurrentExecutions")


def report(state, usage=None):
    result = {**(usage or {}), "stopped": state["mode"] != "running", "mode": state["mode"],
              "reason": state["reason"], "since": state["since"], "recoveryChecks": state["successes"],
              "distributionEnabled": distribution_enabled(), "requestLimit": REQUEST_LIMIT,
              "resumeConcurrency": RESUME_CONCURRENCY, "computeLimitGBSeconds": COMPUTE_LIMIT,
              "cloudFrontRequestLimit": CLOUDFRONT_REQUEST_LIMIT, "cloudFrontBytesLimit": CLOUDFRONT_BYTES_LIMIT}
    print(json.dumps(result))
    return result


def resume_concurrency(value):
    """Reserved concurrency restored on resume. Bounded so a typo cannot open the demo wide."""
    try:
        number = int(value)
    except (TypeError, ValueError):
        raise ValueError("RESUME_CONCURRENCY must be an integer") from None
    if not 1 <= number <= MAX_RESUME_CONCURRENCY:
        raise ValueError("RESUME_CONCURRENCY must be between 1 and %d" % MAX_RESUME_CONCURRENCY)
    return number


RESUME_CONCURRENCY = resume_concurrency(os.environ.get("RESUME_CONCURRENCY", "1"))


def client(service, region):
    # Imported here so the estimate math can be tested without Boto3 installed.
    import boto3
    from botocore.config import Config

    config = Config(connect_timeout=4, read_timeout=8, retries={"max_attempts": 2})
    return boto3.client(service, region_name=region, config=config)


def set_concurrency(value):
    client("lambda", REGION).put_function_concurrency(
        FunctionName=TARGET, ReservedConcurrentExecutions=value
    )


def distribution_enabled():
    if not DISTRIBUTION_ID:
        return None
    response = client("cloudfront", CLOUDFRONT_REGION).get_distribution_config(Id=DISTRIBUTION_ID)
    return response["DistributionConfig"]["Enabled"]


def set_distribution_enabled(enabled):
    """Enable or disable only this stack's distribution. Returns True when an update was sent."""
    if not DISTRIBUTION_ID:
        return False
    cf = client("cloudfront", CLOUDFRONT_REGION)
    current = cf.get_distribution_config(Id=DISTRIBUTION_ID)
    config = current["DistributionConfig"]
    if config["Enabled"] == enabled:
        return False
    config["Enabled"] = enabled
    cf.update_distribution(Id=DISTRIBUTION_ID, IfMatch=current["ETag"], DistributionConfig=config)
    return True


def stop_demo():
    set_concurrency(0)  # First: immediate, and it stops all Lambda cost.
    set_distribution_enabled(False)  # A disabled distribution serves (and bills) no requests.


def cloudfront_usage(start, end):
    cf = client("cloudfront", CLOUDFRONT_REGION)
    cw = client("cloudwatch", CLOUDFRONT_REGION)
    requests = transferred = 0.0
    count = 0
    for page in usage_pages(cf, "list_distributions"):
        for item in page["DistributionList"].get("Items", []):
            dimensions = [{"Name": "DistributionId", "Value": item["Id"]}, {"Name": "Region", "Value": "Global"}]
            requests += metric_sum(cw, "AWS/CloudFront", "Requests", dimensions, start, end)
            transferred += metric_sum(cw, "AWS/CloudFront", "BytesDownloaded", dimensions, start, end)
            count += 1
    return {"cloudFrontRequests": requests, "cloudFrontBytes": transferred, "distributionCount": count}


def compute_gb_seconds(memory_mb, duration_ms, inits):
    """Estimated GB-seconds: invoke Duration plus the Init cap for each estimated cold start."""
    return (duration_ms / 1000 + inits * INIT_CAP_SECONDS) * memory_mb / 1024


def estimate_inits(invocations, evidence):
    """Cold starts to charge. Without trustworthy evidence, every invocation is one."""
    if (evidence is None or not evidence["filter_ok"]
            or evidence["checked_log_events"] < evidence["checked_invocations"]):
        return invocations
    # Recent invocations have no trustworthy Init metric yet. Also charge any
    # discrepancy between independently published Invocation sums conservatively.
    untrusted = max(evidence["recent"], invocations - evidence["before"]
                    - evidence["checked_invocations"], 0)
    starts = max(evidence["init_starts"], 1 if evidence["checked_invocations"] > 0 else 0)
    return evidence["before"] + untrusted + starts


def metric_sum(cw, namespace, metric, dimensions, start, end):
    if end <= start:
        return 0.0
    response = usage_read(cw, "get_metric_statistics",
        Namespace=namespace, MetricName=metric, Dimensions=dimensions,
        StartTime=start, EndTime=end, Period=3600, Statistics=["Sum"],
    )
    return sum(p["Sum"] for p in response.get("Datapoints", []))


def ceil_hour(moment):
    floor = moment.replace(minute=0, second=0, microsecond=0)
    return floor if floor == moment else floor + dt.timedelta(hours=1)


def metered_evidence(logs, cw, function_name, start, end):
    """Cold-start evidence for one function: INIT_START count and a log-delivery cross-check."""
    group = "/aws/lambda/" + function_name
    filters = usage_read(logs, "describe_metric_filters", logGroupName=group, filterNamePrefix=INIT_FILTER_NAME)
    match = [f for f in filters.get("metricFilters", [])
             if f["filterName"] == INIT_FILTER_NAME and f["filterPattern"] == INIT_FILTER_PATTERN
             and any(t["metricNamespace"] == INIT_METRIC_NAMESPACE and t["metricName"] == INIT_METRIC_NAME
                     for t in f["metricTransformations"])]
    if not match:
        return {"filter_ok": False, "before": 0, "recent": 0, "checked_invocations": 0,
                "checked_log_events": 0, "init_starts": 0}
    created = dt.datetime.fromtimestamp(match[0]["creationTime"] / 1000, dt.timezone.utc)
    # Hourly datapoints: round up so the partial hour counts as unmetered (conservative).
    metered_from = min(end, max(start, ceil_hour(created)))
    # Period=3600: trust only complete hours older than the delivery-lag window.
    # Use the same boundaries for all evidence so no invocation hour overlaps.
    checked_until = max(start, (end - LOG_DELIVERY_LAG).replace(
        minute=0, second=0, microsecond=0))
    recent_from = max(metered_from, checked_until)
    function = [{"Name": "FunctionName", "Value": function_name}]
    group_dimension = [{"Name": "LogGroupName", "Value": group}]
    return {
        "filter_ok": True,
        "before": metric_sum(cw, "AWS/Lambda", "Invocations", function, start, metered_from),
        "recent": metric_sum(cw, "AWS/Lambda", "Invocations", function, recent_from, end),
        "checked_invocations": metric_sum(cw, "AWS/Lambda", "Invocations", function,
                                          metered_from, checked_until),
        "checked_log_events": metric_sum(cw, "AWS/Logs", "IncomingLogEvents", group_dimension,
                                         metered_from, checked_until),
        "init_starts": metric_sum(cw, INIT_METRIC_NAMESPACE, INIT_METRIC_NAME, [],
                                  metered_from, checked_until),
    }


def region_usage(region, start, end):
    lam = client("lambda", region)
    cw = client("cloudwatch", region)
    requests = compute = 0.0
    count = 0
    demo = None
    for page in usage_pages(lam, "list_functions"):
        for function in page["Functions"]:
            name = function["FunctionName"]
            dimensions = [{"Name": "FunctionName", "Value": name}]
            invocations = metric_sum(cw, "AWS/Lambda", "Invocations", dimensions, start, end)
            duration = metric_sum(cw, "AWS/Lambda", "Duration", dimensions, start, end)
            evidence = None
            if name == TARGET and region == REGION:
                evidence = metered_evidence(client("logs", region), cw, name, start, end)
            inits = estimate_inits(invocations, evidence)
            if evidence is not None:
                demo = {**evidence, "chargedColdStarts": inits}
            requests += invocations
            # An estimate, not an authoritative billing meter.
            compute += compute_gb_seconds(function["MemorySize"], duration, inits)
            count += 1
    return requests, compute, count, demo


def collect_usage():
    now = dt.datetime.now(dt.timezone.utc)
    start = now.replace(day=1, hour=0, minute=0, second=0, microsecond=0)
    regions = [r["RegionName"] for r in usage_read(client("ec2", REGION), "describe_regions")["Regions"]]
    requests = compute = count = 0
    demo = None
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        futures = [pool.submit(region_usage, region, start, now) for region in regions]
        for future in concurrent.futures.as_completed(futures):
            r, c, n, d = future.result()  # Any missing region/metric access fails closed.
            requests += r
            compute += c
            count += n
            demo = d or demo
    return {"month": start.strftime("%Y-%m"), "requests": requests,
            "computeEstimateGBSeconds": round(compute, 3),
            "functionCount": count, "regionCount": len(regions), "demoColdStarts": demo,
            **cloudfront_usage(start, now)}


def breached(usage):
    return (usage["requests"] >= REQUEST_LIMIT
            or usage["computeEstimateGBSeconds"] >= COMPUTE_LIMIT
            or usage["cloudFrontRequests"] >= CLOUDFRONT_REQUEST_LIMIT
            or usage["cloudFrontBytes"] >= CLOUDFRONT_BYTES_LIMIT)


def handler(event, context):
    state = None
    action = "check"
    try:
        if not isinstance(event, dict):
            raise ValueError("guard event must be an object")
        action = event.get("action", "check")
        if action not in {"check", "status", "stop", "resume"}:
            raise ValueError("action must be check, status, stop, or resume")
        deployment_id = event.get("deploymentId")
        if event.get("deployment") is True and not valid_deployment_id(deployment_id):
            raise ValueError("deployment requires a unique 32-character hexadecimal run ID")
        try:
            state = load_state()
        except MissingState:
            state = state_record("locked", "unknown_state")
            if action not in {"resume", "stop"} and not event.get("Records"):
                # First install or migration: absence is expected and remains
                # stopped, without creating an error alarm that races resume.
                state = stop_with_state(state, "locked", "unknown_state")
                return report(state)
        except StateError:
            if action not in {"resume", "stop"}:
                raise
            state = state_record("locked", "unknown_state")
        reason, signals = signal_reason(event, state)
        if event.get("Records"):
            if reason:
                state = stop_with_state(state, "locked", reason, signals)
            return report(state)
        if action == "stop":
            if event.get("deployment") is True:
                if state["mode"] == "locked":
                    save_state(state)
                    stop_demo()  # Preserve a prior lock, including its reason.
                elif ((state["mode"] == "running" and (
                        current_concurrency() != RESUME_CONCURRENCY or not distribution_enabled()))
                      or (state["mode"] == "paused" and (
                          current_concurrency() != 0 or distribution_enabled()))):
                    state = stop_with_state(state, "locked", "unrecorded_stop")
                else:
                    state = stop_with_state(state, "locked", "deployment_stop", deployment_id=deployment_id)
                return {**report(state), "resumeAllowed": deployment_matches(state, deployment_id)}
            state = stop_with_state(state, "locked", "manual_stop")
            return report(state)
        if (action == "resume" and event.get("deployment") is True
                and not deployment_matches(state, deployment_id)):
            stop_demo()
            return {**report(state), "resumeAllowed": False}
        if state["mode"] == "running" and (current_concurrency() != RESUME_CONCURRENCY or not distribution_enabled()):
            state = stop_with_state(state, "locked", "unrecorded_stop")
        elif state["mode"] == "paused" and (current_concurrency() != 0 or distribution_enabled()):
            state = stop_with_state(state, "locked", "unrecorded_stop")
        elif state["mode"] == "locked":
            stop_demo()
        try:
            usage = collect_usage()
            if breached(usage):
                state = stop_with_state(state, "locked", "usage_limit")
            elif action == "resume":
                blocker = recovery_blocker()
                if blocker == "control_alarm" and event.get("deployment") is True:
                    # A pending OK observation does not replace deployment intent.
                    # An actual SNS stop still replaces it and prevents retry.
                    return {**report(state, usage), "reason": blocker}
                state = (stop_with_state(state, "locked", blocker) if blocker
                         else resume_demo(state, "operator_resume"))
            elif state["mode"] == "paused" and action == "check":
                checked = recovery_check_time(event, state)
                if checked:
                    if (state["lastCheck"] and timestamp(checked) - timestamp(state["lastCheck"])
                            >= CHECK_INTERVAL * 2):
                        state = {**state, "successes": 0, "lastCheck": None}
                    if state["successes"] == 1:
                        blocker = recovery_blocker()
                        state = (stop_with_state(state, "locked", blocker) if blocker
                                 else resume_demo(state, "monitoring_recovered"))
                    else:
                        state = {**state, "successes": 1, "lastCheck": checked}
                        save_state(state)
        except UsageReadError as error:
            print(json.dumps({"event": "usage_read_failed", **error.details}))
            if error.temporary and state["mode"] in {"running", "paused"} and action != "resume":
                state = stop_with_state(state, "paused", "monitoring_failure")
                return report(state)
            if error.temporary and state["mode"] == "locked" and action != "resume":
                stop_demo()
                return report(state)
            raise
        return report(state, usage)
    except Exception as error:
        failed = state_record("locked", "guard_failure", state)
        print(json.dumps({"stopped": True, "reason": "guard operation failed", "errorType": type(error).__name__}))
        persisted = False
        try:
            save_state(failed)
            persisted = True
        except Exception as state_error:
            print(json.dumps({"event": "state_write_failed", "errorType": type(state_error).__name__}))
        finally:
            stop_demo()
        # Without readable/persisted state we cannot deduplicate notices. The
        # control alarm independently notifies the owner on its state change.
        if persisted and state and (state["mode"], state["reason"]) != ("locked", "guard_failure"):
            notify("recovery_failed" if action == "resume" or state["mode"] == "paused"
                   else "guard_failed", failed)
        raise
