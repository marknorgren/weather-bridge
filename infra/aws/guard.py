"""Account-wide usage guard; only the named demo function can be stopped."""
import concurrent.futures
import datetime as dt
import json
import os

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
    for page in cf.get_paginator("list_distributions").paginate():
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
    response = cw.get_metric_statistics(
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
    filters = logs.describe_metric_filters(logGroupName=group, filterNamePrefix=INIT_FILTER_NAME)
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
    for page in lam.get_paginator("list_functions").paginate():
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
    regions = [r["RegionName"] for r in client("ec2", REGION).describe_regions()["Regions"]]
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
    action = "stop" if any(r.get("EventSource") == "aws:sns" for r in event.get("Records", [])) else event.get("action", "check")
    if action not in {"check", "status", "stop", "resume"}:
        raise ValueError("action must be check, status, stop, or resume")
    if action == "stop":
        stop_demo()
        result = {"stopped": True, "reason": "stop requested"}
    else:
        try:
            usage = collect_usage()
            if breached(usage):
                stop_demo()
                result = {**usage, "stopped": True, "reason": "usage threshold reached"}
            else:
                if action == "resume":
                    set_concurrency(RESUME_CONCURRENCY)
                    set_distribution_enabled(True)
                    enabled = True if DISTRIBUTION_ID else None
                else:
                    enabled = distribution_enabled()
                concurrency = client("lambda", REGION).get_function_concurrency(FunctionName=TARGET)
                result = {**usage,
                          "stopped": concurrency.get("ReservedConcurrentExecutions") == 0 or enabled is False,
                          "distributionEnabled": enabled,
                          "reason": "below thresholds", "requestLimit": REQUEST_LIMIT,
                          "resumeConcurrency": RESUME_CONCURRENCY,
                          "computeLimitGBSeconds": COMPUTE_LIMIT,
                          "cloudFrontRequestLimit": CLOUDFRONT_REQUEST_LIMIT,
                          "cloudFrontBytesLimit": CLOUDFRONT_BYTES_LIMIT}
        except Exception as error:
            print(json.dumps({"stopped": True, "reason": "guard operation failed", "errorType": type(error).__name__}))
            stop_demo()
            raise  # Surface monitor failure in CloudWatch; no silent success.
    print(json.dumps(result))
    return result
