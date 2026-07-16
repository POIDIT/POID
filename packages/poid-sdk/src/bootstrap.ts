/**
 * The sandbox entry point. The host bundles this to a self-executing IIFE and
 * injects it as the first script in the application document, so `window.poid`
 * is installed before any application code runs. There are no exports: loading
 * the bundle *is* the installation.
 */

import { installPoid } from "./index.js";

installPoid();
