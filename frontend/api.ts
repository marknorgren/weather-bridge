import createClient from 'openapi-fetch';
import type { paths } from './schema.d.ts';

export class GatewayError extends Error {
    readonly status: number;
    constructor(status: number) {
        super('Weather is temporarily unavailable. Try again.');
        this.name = 'GatewayError';
        this.status = status;
    }
}

/** Keep gateway HTML/empty errors actionable without concealing invalid successes. */
export function createWeatherClient(baseUrl: string, fetcher: typeof fetch = fetch) {
    const client = createClient<paths>({ baseUrl, fetch: fetcher });
    client.use({
        async onResponse({ response }) {
            if (response.ok) return response;
            try {
                await response.clone().json();
                return response;
            } catch {
                throw new GatewayError(response.status);
            }
        },
    });
    return client;
}

/** Aborting alone cannot stop a response that has already completed. */
export class LatestRequest {
    private controller: AbortController | null = null;

    start() {
        this.cancel();
        this.controller = new AbortController();
        return this.controller;
    }

    cancel() {
        this.controller?.abort();
        this.controller = null;
    }

    isCurrent(controller: AbortController) {
        return this.controller === controller && !controller.signal.aborted;
    }
}
