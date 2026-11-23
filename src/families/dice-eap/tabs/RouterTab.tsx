import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState, type CSSProperties } from "react";
import type { DeviceInfo, RouterDestination, RouterPort, RouterState } from "../../../types";
import { matrixStore, useMatrixStore } from "../matrixStore";

type PendingRoute = { destinationId: string; sourceId: string | null };

function groupPorts(ports: RouterPort[]): [string, RouterPort[]][] {
  const groups = new Map<string, RouterPort[]>();
  for (const port of ports) {
    const group = groups.get(port.group) ?? [];
    group.push(port);
    groups.set(port.group, group);
  }
  return [...groups.entries()];
}

function peakWidth(peakDb: number | null): number {
  if (peakDb === null || !Number.isFinite(peakDb)) return 0;
  return Math.max(0, Math.min(100, 100 * 10 ** (peakDb / 20)));
}

function peakClass(peakDb: number | null): string {
  if (peakDb === null) return "is-quiet";
  if (peakDb >= -6) return "is-red";
  if (peakDb >= -18) return "is-yellow";
  return "is-green";
}

function routeClass(destination: RouterDestination): string {
  if (!destination.source_id) return "is-disconnected";
  if (destination.source_id.startsWith("Mixer:")) return "is-mixer-route";
  if (/^(Ins0|Ins1|Aes|Adat):/.test(destination.source_id)) return "is-direct-route";
  return "";
}

export function RouterTab({ active, device, node }: { active: boolean; device: DeviceInfo; node: string }) {
  const { routerState } = useMatrixStore();
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [pendingSelections, setPendingSelections] = useState<Record<string, string>>({});
  const writes = useRef(new Map<string, PendingRoute>());
  const writing = useRef(false);
  const activeState = routerState?.node === node ? routerState : null;

  useEffect(() => {
    if (!active) return;
    let disposed = false;
    void invoke<RouterState>("get_router_state", { node })
      .then((state) => {
        if (disposed) return;
        matrixStore.setRouterState(state);
        setError("");
      })
      .catch((reason: unknown) => {
        if (!disposed) setError(reason instanceof Error ? reason.message : String(reason));
      })
      .finally(() => {
        if (!disposed) setLoading(false);
      });
    return () => {
      disposed = true;
    };
  }, [active, node]);

  async function drainWrites() {
    if (writing.current) return;
    writing.current = true;
    try {
      while (writes.current.size > 0) {
        const [destinationId, route] = writes.current.entries().next().value as [string, PendingRoute];
        writes.current.delete(destinationId);
        try {
          const state = await invoke<RouterState>("set_route", {
            node,
            destinationId: route.destinationId,
            sourceId: route.sourceId,
          });
          matrixStore.setRouterState(state);
          matrixStore.persist();
          setError("");
        } catch (reason) {
          setError(reason instanceof Error ? reason.message : String(reason));
          try {
            const state = await invoke<RouterState>("get_router_state", { node });
            matrixStore.setRouterState(state);
          } catch (recoveryReason) {
            const recoveryError = recoveryReason instanceof Error ? recoveryReason.message : String(recoveryReason);
            setError((current) => `${current}; state refresh failed: ${recoveryError}`);
          }
        } finally {
          setPendingSelections((current) => {
            if (writes.current.has(destinationId)) return current;
            const next = { ...current };
            delete next[destinationId];
            return next;
          });
        }
      }
    } finally {
      writing.current = false;
      if (writes.current.size > 0) void drainWrites();
    }
  }

  function changeRoute(destinationId: string, value: string) {
    const route = { destinationId, sourceId: value || null };
    writes.current.set(destinationId, route);
    setPendingSelections((current) => ({ ...current, [destinationId]: value }));
    void drainWrites();
  }

  if (loading && !activeState) {
    return <div className="router-state-message">Reading DICE router…</div>;
  }
  if (!activeState) {
    return (
      <div className="router-state-message router-state-message--error" role="alert">
        <strong>Router unavailable</strong>
        <span>{error || "No DICE router state is available."}</span>
        <code>{node}</code>
      </div>
    );
  }

  const sourceGroups = groupPorts(activeState.sources);
  const destinationGroups = groupPorts(activeState.destinations.map((destination) => destination.port));
  const destinationsById = new Map(activeState.destinations.map((destination) => [destination.port.id, destination]));
  const routedCount = activeState.destinations.filter((destination) => destination.source_id !== null).length;

  return (
    <section className="router-tab" aria-label={`${device.name} router`}>
      <header className="router-toolbar">
        <div className="router-heading">
          <span className="eyebrow">{device.nickname || device.name}</span>
          <strong>Signal routing</strong>
        </div>
        <span className="router-route-count">{routedCount} / {activeState.destinations.length} connected</span>
        {error && <span className="router-error" role="alert">{error}</span>}
      </header>
      <div className="router-groups">
        {destinationGroups.map(([groupName, ports]) => (
          <section className="router-group" key={groupName} aria-label={groupName}>
            <header className="router-group-heading">
              <h2>{groupName}</h2>
              <span>{ports.length}</span>
            </header>
            <div className="router-destination-grid">
              {ports.map((port) => {
                const destination = destinationsById.get(port.id);
                if (!destination) return null;
                const pending = pendingSelections[port.id];
                const selectedSource = pending ?? destination.source_id ?? "";
                const meterStyle = {
                  "--peak-width": `${peakWidth(destination.peak_db)}%`,
                } as CSSProperties;
                const accentClass = routeClass(destination);
                return (
                  <article className={`router-destination ${accentClass}`} key={port.id}>
                    <div className="router-destination-label" title={port.label}>
                      <span>{port.label}</span>
                      <span className="router-route-indicator" aria-hidden="true" />
                    </div>
                    <div
                      className="router-meter"
                      role="meter"
                      aria-label={`${port.label} peak`}
                      aria-valuemin={-60}
                      aria-valuemax={0}
                      aria-valuenow={destination.peak_db ?? -60}
                    >
                      <span className={`router-meter-fill ${peakClass(destination.peak_db)}`} style={meterStyle} />
                    </div>
                    <select
                      aria-label={`Source for ${port.label}`}
                      className={selectedSource ? "router-source-select" : "router-source-select is-disconnected"}
                      value={selectedSource}
                      onChange={(event) => changeRoute(port.id, event.currentTarget.value)}
                    >
                      <option value="">Disconnected</option>
                      {sourceGroups.map(([sourceGroup, sources]) => (
                        <optgroup label={sourceGroup} key={sourceGroup}>
                          {sources.map((source) => (
                            <option value={source.id} key={source.id}>{source.label}</option>
                          ))}
                        </optgroup>
                      ))}
                    </select>
                  </article>
                );
              })}
            </div>
          </section>
        ))}
      </div>
    </section>
  );
}