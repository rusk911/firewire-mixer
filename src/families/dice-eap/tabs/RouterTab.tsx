import { invoke } from "@tauri-apps/api/core";
import { memo, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import type { DeviceInfo, RouterDestination, RouterPort, RouterRoutesSnapshot, RouterState } from "../../../types";
import { matrixStore } from "../matrixStore";

type PendingRoute = { destinationId: string; sourceId: string | null };
const EMPTY_ROUTER_PORTS: RouterPort[] = [];
const EMPTY_ROUTER_DESTINATIONS: RouterDestination[] = [];

function groupPorts(ports: RouterPort[]): [string, RouterPort[]][] {
  const groups = new Map<string, RouterPort[]>();
  for (const port of ports) {
    const group = groups.get(port.group) ?? [];
    group.push(port);
    groups.set(port.group, group);
  }
  return [...groups.entries()];
}

function routeClass(sourceId: string | null): string {
  if (!sourceId) return "is-disconnected";
  if (sourceId.startsWith("Mixer:")) return "is-mixer-route";
  if (/^(Ins0|Ins1|Aes|Adat):/.test(sourceId)) return "is-direct-route";
  return "";
}

const RouterDestinationCard = memo(function RouterDestinationCard({
  destination,
  sourceGroups,
  selectedSource,
  onRouteChange,
}: {
  destination: RouterDestination;
  sourceGroups: [string, RouterPort[]][];
  selectedSource: string | undefined;
  onRouteChange: (destinationId: string, value: string) => void;
}) {
  const sourceId = useSyncExternalStore(
    (listener) => matrixStore.subscribeDestinationRoute(destination.port.id, listener),
    () => matrixStore.getDestinationRoute(destination.port.id),
    () => matrixStore.getDestinationRoute(destination.port.id),
  );
  const selectedValue = selectedSource ?? sourceId ?? "";
  const hasSignal = useSyncExternalStore(
    (listener) => matrixStore.subscribeDestinationSignal(destination.port.id, listener),
    () => matrixStore.getDestinationSignal(destination.port.id),
    () => matrixStore.getDestinationSignal(destination.port.id),
  );

  return (
    <article className={`router-destination ${routeClass(sourceId)}`}>
      <div className="router-destination-label" title={destination.port.label}>
        <span>{destination.port.label}</span>
        <span className="router-route-indicator" aria-hidden="true" />
      </div>
      <div
        className="router-meter"
        role="meter"
        aria-label={`${destination.port.label} peak`}
        aria-valuemin={0}
        aria-valuemax={1}
        aria-valuenow={hasSignal ? 1 : 0}
      >
        <span className={`router-meter-fill${hasSignal ? " is-active" : ""}`} />
      </div>
      <select
        aria-label={`Source for ${destination.port.label}`}
        className={selectedValue ? "router-source-select" : "router-source-select is-disconnected"}
        value={selectedValue}
        onChange={(event) => onRouteChange(destination.port.id, event.currentTarget.value)}
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
});

function RouterRouteCount({ total }: { total: number }) {
  const connected = useSyncExternalStore(
    matrixStore.subscribeRouterRouteCount,
    matrixStore.getRouterRouteCount,
    matrixStore.getRouterRouteCount,
  );
  return <span className="router-route-count">{connected} / {total} connected</span>;
}

export function RouterTab({ active, device, node }: { active: boolean; device: DeviceInfo; node: string }) {
  const routerState = useSyncExternalStore(
    matrixStore.subscribeRouterState,
    matrixStore.getRouterState,
    matrixStore.getRouterState,
  );
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [pendingSelections, setPendingSelections] = useState<Record<string, string>>({});
  const writes = useRef(new Map<string, PendingRoute>());
  const writing = useRef(false);
  const activeState = routerState?.node === node ? routerState : null;
  const sources = activeState?.sources ?? EMPTY_ROUTER_PORTS;
  const destinations = activeState?.destinations ?? EMPTY_ROUTER_DESTINATIONS;
  const sourceGroups = useMemo(() => groupPorts(sources), [sources]);
  const destinationGroups = useMemo(
    () => groupPorts(destinations.map((destination) => destination.port)),
    [destinations],
  );
  const destinationsById = useMemo(
    () => new Map<string, RouterDestination>(destinations.map((destination) => [destination.port.id, destination])),
    [destinations],
  );

  useEffect(() => {
    if (!active) return;
    let disposed = false;
    let refreshing = false;
    const refresh = async () => {
      if (refreshing) return;
      refreshing = true;
      try {
        if (matrixStore.getRouterState()?.node !== node) {
          const initialState = await invoke<RouterState>("get_router_state", { node });
          if (disposed) return;
          matrixStore.setRouterState(initialState);
        }
        const routes = await invoke<RouterRoutesSnapshot>("get_router_routes", { node });
        if (disposed) return;
        if (matrixStore.setRouterRoutes(node, routes)) {
          const state = await invoke<RouterState>("get_router_state", { node });
          if (disposed) return;
          matrixStore.setRouterState(state);
        }
        setError("");
      } catch (reason) {
        if (!disposed) setError(reason instanceof Error ? reason.message : String(reason));
      } finally {
        refreshing = false;
        if (!disposed) setLoading(false);
      }
    };
    void refresh();
    const timer = window.setInterval(() => { void refresh(); }, 2000);
    return () => {
      disposed = true;
      window.clearInterval(timer);
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


  return (
    <section className="router-tab" aria-label={`${device.name} router`}>
      <header className="router-toolbar">
        <div className="router-heading">
          <span className="eyebrow">{device.nickname || device.name}</span>
          <strong>Signal routing</strong>
        </div>
        <RouterRouteCount total={activeState.destinations.length} />
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
                const selectedSource = pendingSelections[port.id];
                return (
                  <RouterDestinationCard
                    key={port.id}
                    destination={destination}
                    sourceGroups={sourceGroups}
                    selectedSource={selectedSource}
                    onRouteChange={changeRoute}
                  />
                );
              })}
            </div>
          </section>
        ))}
      </div>
    </section>
  );
}