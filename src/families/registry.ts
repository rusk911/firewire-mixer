import type { ComponentType } from "react";
import { BebobWorkspace } from "./bebob/BebobWorkspace";
import { DiceWorkspace } from "./dice-eap/DiceWorkspace";
import type { DeviceFamily, WorkspaceProps } from "../types";

export const familyWorkspaceRegistry: Record<DeviceFamily, ComponentType<WorkspaceProps>> = {
  dice_eap: DiceWorkspace,
  bebob: BebobWorkspace,
};