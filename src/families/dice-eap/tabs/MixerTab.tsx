import { invoke } from "@tauri-apps/api/core";
import { useEffect, useId, useRef, useState, type CSSProperties } from "react";
import { createPortal } from "react-dom";
import type { DeviceInfo, MixerChannel, RouterPort, RouterState } from "../../../types";
import { matrixStore, useMatrixStore } from "../matrixStore";

const MIN_DB = -60;

function displayDb(value: number | null): string {
  return value === null ? "−∞" : `${value.toFixed(1)}`;
}

function meterLevel(db: number | null): number {
  return db === null ? 0 : Math.max(0, Math.min(100, (db + 60) / 60 * 100));
}

function PeakMeter({ levels, label }: { levels: (number | null)[]; label: string }) {
  return (
    <div className="strip-meters" aria-label={label}>
      {levels.map((level, index) => (
        <div className="strip-meter" key={index}>
          <div className="strip-meter-fill" style={{ "--level": `${meterLevel(level)}%` } as CSSProperties} />
        </div>
      ))}
    </div>
  );
}

function Fader({ value, max, label, onChange }: {
  value: number | null;
  max: number;
  label: string;
  onChange: (value: number | null) => void;
}) {
  const rail = useRef<HTMLDivElement>(null);
  const wheelValue = useRef(value);
  const changeRef = useRef(onChange);
  const hoverTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const [tooltip, setTooltip] = useState<{ x: number; y: number } | null>(null);
  const tooltipId = useId();
  const progress = Math.max(0, Math.min(1, ((value ?? MIN_DB) - MIN_DB) / (max - MIN_DB)));
  const ticks = [...new Set([max, 0, -12, -24, -36, -48, MIN_DB])].filter((tick) => tick <= max);

  useEffect(() => { wheelValue.current = value; }, [value]);
  useEffect(() => { changeRef.current = onChange; }, [onChange]);
  useEffect(() => {
    const element = rail.current;
    if (!element) return;
    function handleWheel(event: WheelEvent) {
      if (event.deltaY === 0) return;
      event.preventDefault();
      const step = 0.5;
      const next = Math.round(Math.max(MIN_DB, Math.min(max,
        (wheelValue.current ?? MIN_DB) + (event.deltaY < 0 ? step : -step),
      )) * 10) / 10;
      const gain = next <= MIN_DB ? null : next;
      if (gain !== wheelValue.current) {
        wheelValue.current = gain;
        changeRef.current(gain);
      }
    }
    element.addEventListener("wheel", handleWheel, { passive: false });
    return () => element.removeEventListener("wheel", handleWheel);
  }, [max]);
  useEffect(() => () => { if (hoverTimer.current) clearTimeout(hoverTimer.current); }, []);

  function clearTooltip() {
    if (hoverTimer.current) clearTimeout(hoverTimer.current);
    hoverTimer.current = undefined;
    setTooltip(null);
  }

  function queueTooltip(event: React.PointerEvent<HTMLInputElement>) {
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      clearTooltip();
      return;
    }
    const { clientX: x, clientY: y } = event;
    if (hoverTimer.current) clearTimeout(hoverTimer.current);
    setTooltip(null);
    hoverTimer.current = setTimeout(() => setTooltip({ x, y }), 1000);
  }

  function positionFor(event: React.PointerEvent<HTMLInputElement>) {
    const bounds = rail.current?.getBoundingClientRect();
    if (!bounds) return;
    const position = Math.max(0, Math.min(1, (bounds.bottom - 15 - event.clientY) / Math.max(1, bounds.height - 30)));
    const next = Math.round((MIN_DB + position * (max - MIN_DB)) * 10) / 10;
    onChange(next <= MIN_DB ? null : next);
  }

  function modifier(event: React.PointerEvent<HTMLInputElement>) {
    if (event.button !== 0) return;
    clearTooltip();
    event.preventDefault();
    if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) {
      onChange(event.altKey ? null : event.ctrlKey || event.metaKey ? -12 : max);
    } else {
      event.currentTarget.focus();
      event.currentTarget.setPointerCapture(event.pointerId);
      positionFor(event);
    }
  }
  return (
    <div className="strip-fader" ref={rail}>
      <div className="strip-fader-slot" aria-hidden="true" />
      {ticks.map((tick) => {
        const tickProgress = (tick - MIN_DB) / (max - MIN_DB);
        return <span className="strip-fader-tick" key={tick} aria-hidden="true"
          style={{ top: `calc(${(1 - tickProgress) * 100}% + ${30 * tickProgress - 15}px)` }}>
          <span>{tick === MIN_DB ? "−∞" : tick > 0 ? `+${tick}` : tick}</span>
        </span>;
      })}
      <span className="strip-fader-cap" aria-hidden="true"
        style={{ top: `calc(${(1 - progress) * 100}% - ${(1 - progress) * 30}px)` }} />
      <input
        aria-label={label}
        aria-describedby={tooltip ? tooltipId : undefined}
        aria-valuetext={value === null ? "minus infinity" : `${value.toFixed(1)} dB`}
        className="strip-fader-input"
        type="range"
        min={MIN_DB}
        max={max}
        step={0.1}
        value={value ?? MIN_DB}
        onChange={(event) => {
          const next = Number(event.currentTarget.value);
          onChange(next <= MIN_DB ? null : next);
        }}
        onPointerDown={modifier}
        onPointerEnter={queueTooltip}
        onPointerLeave={clearTooltip}
        onPointerMove={(event) => {
          if (event.currentTarget.hasPointerCapture(event.pointerId)) positionFor(event);
          else queueTooltip(event);
        }}
        onPointerUp={(event) => {
          if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
          queueTooltip(event);
        }}
        onPointerCancel={clearTooltip}
        onDoubleClick={(event) => { event.preventDefault(); onChange(0); }}
      />
      {tooltip && createPortal(
        <div id={tooltipId} className="rotary-tooltip" role="tooltip" style={{
          left: Math.max(8, Math.min(tooltip.x + 12, window.innerWidth - 214)),
          top: Math.max(8, Math.min(tooltip.y + 12, window.innerHeight - 145)),
        }}>
          <strong>{displayDb(value)} dB</strong>
          <span>Double-click: 0 dB</span>
          <span>Shift + click: {max > 0 ? "+" : ""}{max} dB</span>
          <span>Ctrl + click: −12 dB</span>
          <span>Alt + click: −∞</span>
          <span>Wheel: 0.5 dB</span>
        </div>,
        document.body,
      )}
    </div>
  );
}

function PanControl({ value, label, onChange }: { value: number; label: string; onChange: (value: number) => void }) {
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState("");
  const [hover, setHover] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => { if (timer.current) clearTimeout(timer.current); }, []);
  const percent = Math.round(value * 100);
  const panPosition = (percent + 100) / 2;
  const panStyle = {
    "--pan-start": `${Math.min(50, panPosition)}%`,
    "--pan-end": `${Math.max(50, panPosition)}%`,
  } as CSSProperties;
  const description = percent === 0 ? "C" : `${Math.abs(percent)}% ${percent < 0 ? "L" : "R"}`;
  function apply() {
    const number = Number(text);
    if (text.trim() && Number.isFinite(number)) onChange(Math.max(-100, Math.min(100, number)) / 100);
    setEditing(false);
  }
  return (
    <div className="strip-pan" onMouseEnter={() => { timer.current = setTimeout(() => setHover(true), 1000); }}
      onMouseLeave={() => { if (timer.current) clearTimeout(timer.current); setHover(false); }}>
      {editing ? (
        <input aria-label={`${label} value`} className="strip-pan-editor" type="number" min="-100" max="100"
          value={text} autoFocus onChange={(event) => setText(event.currentTarget.value)} onBlur={apply}
          onKeyDown={(event) => { if (event.key === "Enter") event.currentTarget.blur(); if (event.key === "Escape") setEditing(false); }} />
      ) : (
        <input aria-label={label} className="strip-pan-slider" style={panStyle} type="range" min="-100" max="100" value={percent}
          onChange={(event) => onChange(Number(event.currentTarget.value) / 100)}
          onDoubleClick={() => onChange(0)}
          onContextMenu={(event) => { event.preventDefault(); setHover(false); setText(String(percent)); setEditing(true); }} />
      )}
      {hover && !editing && <span className="strip-pan-tooltip" role="tooltip">{description}</span>}
    </div>
  );
}

function EditableLabel({ value, label, onSave }: { value: string; label: string; onSave: (value: string) => void }) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  return <input className="strip-label-input" aria-label={label} value={draft} title={value}
    onChange={(event) => setDraft(event.currentTarget.value)}
    onBlur={() => { if (draft !== value) onSave(draft.trim()); }}
    onKeyDown={(event) => { if (event.key === "Enter") event.currentTarget.blur(); if (event.key === "Escape") { setDraft(value); event.currentTarget.blur(); } }} />;
}

function sourceGroups(sources: RouterPort[]): [string, RouterPort[]][] {
  const groups = new Map<string, RouterPort[]>();
  for (const source of sources) {
    const group = groups.get(source.group) ?? [];
    group.push(source);
    groups.set(source.group, group);
  }
  return [...groups.entries()];
}

function SourceSelector({ node, router, col, stereo, label, onError }: {
  node: string; router: RouterState | null; col: number; stereo: boolean; label: string; onError: (message: string) => void;
}) {
  const [busy, setBusy] = useState(false);
  const destinationId = router?.matrix_input_port_ids[col];
  const partnerDestination = stereo ? router?.matrix_input_port_ids[col + 1] : undefined;
  const current = router?.destinations.find((destination) => destination.port.id === destinationId)?.source_id ?? "";
  const right = router?.destinations.find((destination) => destination.port.id === partnerDestination)?.source_id ?? "";
  const sources = router?.sources ?? [];
  const pairs = sources.filter((source) => source.stereo_partner_id && sources.some((candidate) => candidate.id === source.stereo_partner_id));
  const validPair = pairs.some((source) => source.id === current && source.stereo_partner_id === right);
  const selected = stereo && (current || right) && !validPair ? "__custom__" : current;

  async function change(value: string) {
    if (!destinationId || !router) return;
    setBusy(true);
    try {
      let updated = router;
      if (current !== value) {
        updated = await invoke<RouterState>("set_route", { node, destinationId, sourceId: value || null });
        matrixStore.setRouterState(updated);
      }
      if (stereo && partnerDestination) {
        const partner = value ? sources.find((source) => source.id === value)?.stereo_partner_id ?? null : null;
        if (right !== (partner ?? "")) {
          updated = await invoke<RouterState>("set_route", { node, destinationId: partnerDestination, sourceId: partner });
          matrixStore.setRouterState(updated);
        }
      }
      matrixStore.persist();
      onError("");
    } catch (reason) {
      onError(reason instanceof Error ? reason.message : String(reason));
      try {
        matrixStore.setRouterState(await invoke<RouterState>("get_router_state", { node }));
        matrixStore.persist();
      } catch (refreshError) {
        onError(`${String(reason)}; ${String(refreshError)}`);
      }
    } finally {
      setBusy(false);
    }
  }

  return (
    <select aria-label={`Source for ${label}`} className="strip-source" disabled={!destinationId || busy}
      value={selected} title={label} onChange={(event) => void change(event.currentTarget.value)}>
      <option value="">Disconnected</option>
      {selected === "__custom__" && <option value="__custom__" disabled>Custom routing</option>}
      {sourceGroups(stereo ? pairs : sources).map(([group, entries]) => (
        <optgroup key={group} label={group}>
          {entries.map((source) => {
            const partner = stereo ? sources.find((candidate) => candidate.id === source.stereo_partner_id) : undefined;
            return <option key={source.id} value={source.id}>{partner ? `${source.label} / ${partner.label}` : source.label}</option>;
          })}
        </optgroup>
      ))}
    </select>
  );
}

function channelPeak(input: number | null, gain: number | null, channel: MixerChannel): number | null {
  if (input === null) return null;
  if (channel.pfm) return input;
  return gain === null ? null : input + gain;
}

export function MixerTab({ active, device, node }: { active: boolean; device: DeviceInfo; node: string }) {
  const { matrix, mixer, mixerLoaded, mixerError, routerState } = useMatrixStore();
  const [selectedRow, setSelectedRow] = useState(0);
  const [routeError, setRouteError] = useState("");
  const [editingBus, setEditingBus] = useState<number | null>(null);
  const [busName, setBusName] = useState("");
  const router = routerState?.node === node ? routerState : null;

  useEffect(() => {
    if (!active) return;
    let disposed = false;
    void invoke<RouterState>("get_router_state", { node }).then((state) => {
      if (!disposed) matrixStore.setRouterState(state);
    }).catch((reason: unknown) => {
      if (!disposed) setRouteError(reason instanceof Error ? reason.message : String(reason));
    });
    return () => { disposed = true; };
  }, [active, node]);

  if (!matrix || !mixer || !mixerLoaded) {
    return <div className={`matrix-state-message${mixerError ? " matrix-state-message--error" : ""}`} role={mixerError ? "alert" : undefined}>
      {mixerError ? <><strong>Mixer unavailable</strong><span>{mixerError}</span></> : "Reading mixer configuration…"}
    </div>;
  }
  const row = selectedRow < mixer.buses.length ? selectedRow : 0;
  const bus = mixer.buses[row];
  const stereo = bus.stereo && !!mixer.buses[row + 1];
  const sourceNames = matrix.column_labels;
  const tabs = mixer.buses.flatMap((candidate, index) => {
    if (index > 0 && mixer.buses[index - 1].stereo) return [];
    return [{ index, label: candidate.label || (candidate.stereo ? `Mix Out ${index + 1}/${index + 2}` : `Mix Out ${index + 1}`) }];
  });
  const masterLevels = (stereo ? [row, row + 1] : [row]).map((output) => {
    const sourceId = router?.matrix_output_port_ids[output];
    const peaks = router?.destinations.filter((destination) => destination.source_id === sourceId)
      .map((destination) => destination.peak_db).filter((value): value is number => value !== null) ?? [];
    return peaks.length && bus.master_db !== null ? Math.max(...peaks) + bus.master_db : null;
  });

  return (
    <section className="mixer-tab" aria-label={`${device.name} mixer`}>
      <div className="mixer-bus-tabs" role="tablist" aria-label="Mix buses">
        {tabs.map((tab) => editingBus === tab.index ? (
          <input key={tab.index} className="mixer-bus-rename" aria-label="Bus name" autoFocus value={busName}
            onChange={(event) => setBusName(event.currentTarget.value)}
            onBlur={() => { matrixStore.updateBus(tab.index, { label: busName.trim() || null }, false); setEditingBus(null); }}
            onKeyDown={(event) => { if (event.key === "Enter") event.currentTarget.blur(); if (event.key === "Escape") setEditingBus(null); }} />
        ) : (
          <button type="button" role="tab" key={tab.index} aria-selected={row === tab.index}
            className={row === tab.index ? "mixer-bus-tab is-active" : "mixer-bus-tab"}
            onClick={() => setSelectedRow(tab.index)} onDoubleClick={() => { setEditingBus(tab.index); setBusName(mixer.buses[tab.index].label ?? ""); }}
            title={tab.label}>{tab.label}</button>
        ))}
      </div>
      {(mixerError || routeError) && <div className="mixer-error" role="alert">{mixerError || routeError}</div>}
      <div className="mixer-body">
        <div className="mixer-channels">
          {bus.channels.map((channel, col) => {
            if (col > 0 && bus.channels[col - 1].stereo) return null;
            const linked = channel.stereo && !!bus.channels[col + 1];
            const sourceLabel = linked ? `${sourceNames[col]} / ${sourceNames[col + 1]}` : sourceNames[col];
            const levels = (linked ? [col, col + 1] : [col]).map((input, idx) => {
              const targetRow = stereo && linked && idx === 1 ? row + 1 : row;
              return channelPeak(
                matrix.input_peaks_db[input] ?? null,
                matrix.gains[targetRow]?.[input] ?? null,
                channel,
              );
            });
            return (
              <article className={`mixer-strip${linked ? " is-stereo" : ""}`} key={col}>
                <SourceSelector node={node} router={router} col={col} stereo={linked} label={sourceLabel} onError={setRouteError} />
                <div className="strip-top-controls">
                  <button type="button" title="Stereo link" aria-label={`Stereo link ${sourceLabel}`} aria-pressed={linked}
                    disabled={col + 1 >= bus.channels.length || (!linked && !!bus.channels[col + 1]?.stereo)}
                    onClick={() => matrixStore.setChannelStereo(row, col, !linked)}>Stereo</button>
                  <button type="button" title="Pre-Fader Meter" aria-label={`Pre-Fader Meter ${sourceLabel}`} aria-pressed={channel.pfm}
                    onClick={() => matrixStore.updateChannel(row, col, { pfm: !channel.pfm }, false)}>PFM</button>
                </div>
                {stereo ? <PanControl label={`Pan ${sourceLabel}`} value={channel.pan} onChange={(pan) => matrixStore.updateChannel(row, col, { pan })} /> : <div className="strip-pan-spacer" />}
                <div className="strip-fader-area">
                  <Fader label={`Fader ${sourceLabel}`} value={channel.fader_db} max={stereo ? 6 : 12}
                    onChange={(fader_db) => matrixStore.updateChannel(row, col, { fader_db })} />
                  <PeakMeter levels={levels} label={`Level ${sourceLabel}`} />
                </div>
                <output className="strip-value">{displayDb(channel.fader_db)}</output>
                <div className="strip-actions">
                  <button type="button" title="Solo" aria-label={`Solo ${sourceLabel}`} aria-pressed={channel.solo}
                    onClick={() => matrixStore.updateChannel(row, col, { solo: !channel.solo })}>S</button>
                  <button type="button" title="Mute" aria-label={`Mute ${sourceLabel}`} aria-pressed={channel.muted}
                    onClick={() => matrixStore.updateChannel(row, col, { muted: !channel.muted })}>M</button>
                </div>
                <EditableLabel value={channel.label ?? sourceLabel} label={`Channel label ${col + 1}`}
                  onSave={(label) => matrixStore.updateChannel(row, col, { label: label || null }, false)} />
              </article>
            );
          })}
        </div>
        <aside className={`mixer-master${stereo ? " is-stereo" : ""}`} aria-label="Master">
          <div className="master-heading"><span>MASTER</span><strong>{tabs.find((tab) => tab.index === row)?.label}</strong></div>
          <div className="strip-top-controls">
            <button type="button" title="Stereo bus link" aria-pressed={stereo} disabled={row + 1 >= mixer.buses.length || (!stereo && !!mixer.buses[row + 1]?.stereo)}
              onClick={() => matrixStore.setBusStereo(row, !stereo)}>Stereo</button>
          </div>
          <div className="strip-pan-spacer" />
          <div className="strip-fader-area">
            <Fader label="Master fader" value={bus.master_db} max={0} onChange={(master_db) => matrixStore.updateBus(row, { master_db })} />
            <PeakMeter levels={masterLevels} label="Master level" />
          </div>
          <output className="strip-value">{displayDb(bus.master_db)}</output>
          <div className="master-footer">{device.nickname || device.name}</div>
        </aside>
      </div>
    </section>
  );
}