"""Operational endpoints must report the current process, never a cached response."""

import json
from fnmatch import fnmatchcase
from pathlib import Path
import unittest


class DiagnosticCachingTests(unittest.TestCase):
    def distribution_config(self):
        template = json.loads(Path(__file__).with_name("template.json").read_text())
        distribution = next(
            resource for resource in template["Resources"].values()
            if resource["Type"] == "AWS::CloudFront::Distribution"
        )
        return distribution["Properties"]["DistributionConfig"]

    def behavior_for(self, path):
        config = self.distribution_config()
        return next(
            (behavior for behavior in config["CacheBehaviors"]
             if fnmatchcase(path, behavior["PathPattern"])),
            config["DefaultCacheBehavior"],
        )

    def test_operational_endpoints_use_the_managed_disabled_cache_policy(self):
        # template.json groups these seven-character root paths to fit the Free plan.
        for path in ("/healthz", "/version", "/metrics"):
            with self.subTest(path=path):
                self.assertEqual(
                    self.behavior_for(path)["CachePolicyId"],
                    "4135ea2d-6df8-44a3-9df3-4b5a84be39ad",
                )

    def test_free_plan_stays_within_five_total_cache_behaviors(self):
        config = self.distribution_config()
        self.assertLessEqual(1 + len(config["CacheBehaviors"]), 5)

    def test_diagnostic_pattern_preserves_pages_assets_and_api_routing(self):
        for path in ("/", "/developer", "/openapi.json", "/assets/weather.js"):
            with self.subTest(path=path):
                self.assertEqual(
                    self.behavior_for(path)["CachePolicyId"],
                    "658327ea-f89d-4fab-a63d-7e88639e58f6",
                )
        for path, pattern in (("/mcp", "/mcp*"), ("/v1/weather", "/v1/*")):
            with self.subTest(path=path):
                self.assertEqual(self.behavior_for(path)["PathPattern"], pattern)


if __name__ == "__main__":
    unittest.main()
