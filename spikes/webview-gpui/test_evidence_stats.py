import unittest
from evidence_stats import latency_stats, summarize_events

class EvidenceStatsTests(unittest.TestCase):
    def test_tail_threshold_and_nearest_rank(self):
        result=latency_stats([1,16,16.01,114])
        self.assertEqual(result["over_16ms_count"],2)
        self.assertEqual(result["over_16ms_percent"],50)
        self.assertEqual(result["p99_ms"],114)
    def test_untyped_legacy_is_not_ping(self):
        result=summarize_events([{"event":"roundtrip","data":{"ms":1}}])
        self.assertEqual(result["evidence_class"],"legacy_unversioned")
        self.assertEqual(result["by_kind"]["unknown"]["count"],1)
        self.assertNotIn("pong",result["by_kind"])
    def test_empty_has_no_percentile(self):
        self.assertIsNone(latency_stats([])["p99_ms"])

if __name__=="__main__":unittest.main()
