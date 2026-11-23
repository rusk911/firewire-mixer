import { useEffect, useRef, useState } from "react";
import type { DeviceInfo, DiceGlobalState } from "../../types";

interface DiceHeaderProps {
  device: DeviceInfo;
  state: DiceGlobalState;
  onNicknameSave: (name: string) => Promise<void>;
  onClockSourceSelect: (source: string) => void;
  onSampleRateSelect: (rate: number) => void;
}

export function DiceHeader({
  device,
  state,
  onNicknameSave,
  onClockSourceSelect,
  onSampleRateSelect,
}: DiceHeaderProps) {
  const [nickname, setNickname] = useState(state.nickname);
  const [saving, setSaving] = useState(false);
  const savingNickname = useRef<string | null>(null);

  useEffect(() => setNickname(state.nickname), [state.nickname]);

  async function saveNickname() {
    if (nickname === state.nickname || savingNickname.current === nickname) return;
    const value = nickname;
    savingNickname.current = value;
    setSaving(true);
    try {
      await onNicknameSave(value);
    } finally {
      savingNickname.current = null;
      setSaving(false);
    }
  }

  function submitNickname(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    void saveNickname();
  }

  return (
    <header className="dice-header">
      <div className="device-identity">
        <span className="device-family-mark" aria-hidden="true">D</span>
        <div className="identity-copy">
          <span className="eyebrow">DICE EAP · {device.node_path}</span>
          <strong title={device.name}>{device.name}</strong>
        </div>
      </div>

      <form className="nickname-control" onSubmit={submitNickname}>
        <label htmlFor="device-nickname">Nickname</label>
        <input
          id="device-nickname"
          disabled={saving}
          maxLength={64}
          onBlur={() => void saveNickname()}
          onChange={(event) => setNickname(event.target.value)}
          value={nickname}
        />
      </form>

      <label className="header-select">
        <span>Clock source</span>
        <select onChange={(event) => onClockSourceSelect(event.target.value)} value={state.clock_source}>
          {state.clock_sources.map((source) => (
            <option key={source.value} value={source.value}>{source.label}</option>
          ))}
        </select>
      </label>

      <label className="header-select sample-rate-select">
        <span>Sample rate</span>
        <select
          onChange={(event) => onSampleRateSelect(Number(event.target.value))}
          value={state.sample_rate ?? ""}
        >
          {!state.sample_rate && <option value="">Unavailable</option>}
          {state.sample_rates.map((rate) => (
            <option key={rate} value={rate}>{(rate / 1000).toFixed(rate % 1000 ? 1 : 0)} kHz</option>
          ))}
        </select>
      </label>

      <div className="device-indicators" aria-label="Device status">
        <span className="indicator">
          <i className={state.clock_locked ? "status-led is-on" : "status-led"} />
          <span>Sync</span>
        </span>
        <span className="indicator">
          <i className={state.stream_active ? "status-led is-on" : "status-led"} />
          <span>Stream</span>
        </span>
      </div>
    </header>
  );
}