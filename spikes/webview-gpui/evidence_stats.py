"""Version-aware latency statistics. Missing kind is unknown, never assumed ping."""
import math

def latency_stats(values):
    values = sorted(values)
    def percentile(p):
        return values[max(0, math.ceil(len(values)*p)-1)] if values else None
    count = len(values)
    over = sum(v > 16 for v in values)
    return {"count":count,"p50_ms":percentile(.5),"p99_ms":percentile(.99),
            "max_ms":max(values) if values else None,"over_16ms_count":over,
            "over_16ms_percent":100*over/count if count else None}

def summarize_events(events):
    starts=[e["data"] for e in events if e["event"]=="run_start"]
    replies=[e["data"] for e in events if e["event"]=="roundtrip"]
    return {"provenance":starts[0] if len(starts)==1 else None,
            "evidence_class":"versioned" if len(starts)==1 and starts[0].get("schema")==2 else "legacy_unversioned",
            "first_render_ms":[e["data"]["ms"] for e in events if e["event"]=="first_render"],
            "ready":[e["data"] for e in events if e["event"]=="ready"],
            "key_route_count":sum(e["event"]=="key_route" for e in events),
            "all_replies":latency_stats([e["ms"] for e in replies]),
            "by_kind":{kind:latency_stats([e["ms"] for e in replies if e.get("kind","unknown")==kind])
                       for kind in sorted({e.get("kind","unknown") for e in replies})}}
