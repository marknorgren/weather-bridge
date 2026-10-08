// Generated from openapi.json by pnpm run generate. Do not edit.
export interface paths {
    "/metrics": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Bounded Prometheus operational metrics with fixed labels */
        get: operations["getMetrics"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/alerts": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * Active official alerts; inspect alertsStatus
         * @description Supply exactly one location mode: city, cityId, or both lat and lon. Fetches only NWS active alerts, independent of forecast availability. alertsStatus unavailable means alerts could not be checked, not that there are none. Cached up to 120 seconds. US/territory coverage.
         */
        get: operations["getActiveAlerts"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/cities": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Search US city names or prefixes; optional state qualifier */
        get: operations["searchCities"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/forecast/hourly": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * Next 24 hourly forecast periods
         * @description Supply exactly one location mode: city, cityId, or both lat and lon. Fetches only the NWS grid lookup and hourly forecast; alerts are not checked (alertsStatus is not-checked). Hourly data are cached up to 120 seconds and grid lookups up to six hours. US/territory coverage.
         */
        get: operations["getHourlyForecast"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/v1/weather": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /**
         * Weather by city or coordinates
         * @description Supply exactly one location mode: city, cityId, or both lat and lon. Forecasts, observations and alerts are cached up to 120 seconds and NWS grid lookups up to six hours; city centers approximate a point. US/territory coverage.
         */
        get: operations["getWeather"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
    "/version": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        /** Running version and embedded source revision */
        get: operations["getVersion"];
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
}
export type webhooks = Record<string, never>;
export interface components {
    schemas: {
        /** @description Active alerts and whether they could be checked. */
        ActiveAlerts: {
            alerts: components["schemas"]["Alert"][];
            /** @description unavailable means alerts could not be checked, not that there are none. */
            alertsStatus: components["schemas"]["CheckedAlertsStatus"];
            /**
             * Format: date-time
             * @description When this response was assembled, not when upstream data were observed.
             */
            assembledAt: string;
            /**
             * Format: uint64
             * @description Upstream responses are cached up to this many seconds.
             */
            cacheMaxAgeSeconds: number;
            location: components["schemas"]["Location"];
            sources: components["schemas"]["AlertSources"];
            units: components["schemas"]["Units"];
            /** @description Human-readable notes. Source failures make the response partial (Cache-Control: no-store). */
            warnings: string[];
        };
        /** @description `{ "data": ..., "meta": ... }` wrapper for every successful response. */
        ActiveAlertsEnvelope: {
            data: components["schemas"]["ActiveAlerts"];
            meta: components["schemas"]["Meta"];
        };
        /** @description One active NWS alert. Text fields are the original NWS wording. */
        Alert: {
            area: string | null;
            description: string | null;
            /** Format: date-time */
            effectiveAt: string | null;
            event: string | null;
            /** Format: date-time */
            expiresAt: string | null;
            headline: string | null;
            id: string | null;
            /** @description Original NWS instruction text. */
            instruction: string | null;
            severity: string | null;
        };
        AlertSources: {
            alerts: components["schemas"]["QuerySource"];
        };
        Attribution: {
            /** @description Present when the source requires a license notice. */
            license?: string | null;
            name: string;
            url: string;
        };
        /** @description Alert check outcomes for endpoints that check alerts. */
        CheckedAlertsStatus: "checked" | "unavailable";
        /** @description Up to ten city search matches, ordered by population. */
        Cities: components["schemas"]["City"][];
        /** @description `{ "data": ..., "meta": ... }` wrapper for every successful response. */
        CitiesEnvelope: {
            data: components["schemas"]["Cities"];
            meta: components["schemas"]["Meta"];
        };
        /** @description A city from the GeoNames-derived index (CC BY 4.0). */
        City: {
            asciiName: string;
            country: string;
            /**
             * Format: uint64
             * @description GeoNames ID; use as cityId.
             */
            id: number;
            /** Format: double */
            latitude: number;
            /** Format: double */
            longitude: number;
            name: string;
            /** Format: uint64 */
            population: number;
            /** @description Two-letter state or territory code. */
            state: string;
            stateName: string;
            timeZone: string;
        };
        /** @description Wire shape of an error response: `{"errors": [ErrorObject]}`. */
        ErrorBody: {
            errors: components["schemas"]["ErrorObject"][];
        };
        /** @description Each code has one HTTP status: INVALID_LOCATION 400, FORBIDDEN 403, CITY_NOT_FOUND 404, AMBIGUOUS_CITY 409, REQUEST_TOO_LARGE 413, OUTSIDE_COVERAGE 422, UPSTREAM_UNAVAILABLE 502, BUSY 503, UPSTREAM_TIMEOUT 504. */
        ErrorCode: "INVALID_LOCATION" | "CITY_NOT_FOUND" | "AMBIGUOUS_CITY" | "OUTSIDE_COVERAGE" | "UPSTREAM_UNAVAILABLE" | "UPSTREAM_TIMEOUT" | "BUSY" | "FORBIDDEN" | "REQUEST_TOO_LARGE";
        ErrorObject: {
            /** @description Candidates for AMBIGUOUS_CITY and suggestions for CITY_NOT_FOUND; otherwise null. */
            choices: components["schemas"]["City"][] | null;
            code: components["schemas"]["ErrorCode"];
            detail: string;
            /** @description HTTP status as a string, e.g. "409". */
            status: string;
        };
        /** @description Point sent to the NWS /points grid lookup: the location rounded to two decimals (about 1 km) so nearby requests share the grid lookup and its forecasts. Alerts use four-decimal coordinate precision rather than this grid point. */
        GridPoint: {
            /** Format: double */
            latitude: number;
            /** Format: double */
            longitude: number;
        };
        /**
         * @description The hourly endpoint deliberately does not check alerts.
         * @enum {string}
         */
        HourlyAlertsStatus: "not-checked";
        /** @description Next 24 hourly forecast periods. Alerts are not checked. */
        HourlyForecast: {
            /** @description This endpoint does not check alerts; use /v1/alerts. */
            alertsStatus: components["schemas"]["HourlyAlertsStatus"];
            /**
             * Format: date-time
             * @description When this response was assembled, not when upstream data were observed.
             */
            assembledAt: string;
            /**
             * Format: uint64
             * @description Upstream responses are cached up to this many seconds.
             */
            cacheMaxAgeSeconds: number;
            hourly: components["schemas"]["Period"][];
            location: components["schemas"]["Location"];
            sources: components["schemas"]["HourlySources"];
            units: components["schemas"]["Units"];
            /** @description Always includes a fixed note that alerts were not checked; that note alone does not make the response partial. */
            warnings: string[];
        };
        /** @description `{ "data": ..., "meta": ... }` wrapper for every successful response. */
        HourlyForecastEnvelope: {
            data: components["schemas"]["HourlyForecast"];
            meta: components["schemas"]["Meta"];
        };
        HourlySources: {
            hourly: components["schemas"]["IssuedSource"];
        };
        /** @description An NWS document and when it was issued. */
        IssuedSource: {
            /**
             * Format: date-time
             * @description NWS update time; null when unavailable.
             */
            issuedAt: string | null;
            url: string;
        };
        Location: {
            /**
             * Format: uint64
             * @description GeoNames ID when the location came from the city index.
             */
            cityId: number | null;
            /** @description The rounded point sent to the NWS /points grid lookup; forecasts follow that grid. */
            gridLookupPoint: components["schemas"]["GridPoint"];
            /**
             * Format: double
             * @description City center, or the latitude supplied.
             */
            latitude: number;
            /**
             * Format: double
             * @description City center, or the longitude supplied.
             */
            longitude: number;
            name: string;
            precision: components["schemas"]["Precision"];
            timeZone: string | null;
        };
        Meta: {
            /** @description Data sources that must be credited. */
            attribution: components["schemas"]["Attribution"][];
        };
        /** @description Station observation, never a forecast. Missing quantities are null. */
        Observation: {
            /**
             * Format: int64
             * @description Seconds since observedAt; null when the timestamp is invalid.
             */
            ageSeconds: number | null;
            condition: string | null;
            humidityPercent: number | null;
            /** Format: date-time */
            observedAt: string;
            /** @description NWS URL of this observation. */
            sourceUrl: string;
            /** @description True when over 2 hours old or the timestamp is invalid. */
            stale: boolean;
            /** @description NWS station identifier. */
            station: string;
            /** Format: double */
            stationDistanceKm: number;
            temperature: components["schemas"]["Quantity"] | null;
            windDirectionDegrees: number | null;
            windSpeed: components["schemas"]["Quantity"] | null;
        };
        /** @description One NWS forecast period (daily or hourly). */
        Period: {
            condition: string | null;
            /** @description Official NWS forecast text, verbatim; measurements keep the original NWS units. */
            detail: string | null;
            /** Format: date-time */
            endsAt: string | null;
            isDaytime: boolean | null;
            name: string | null;
            precipitationProbabilityPercent: number | null;
            /** Format: date-time */
            startsAt: string | null;
            temperature: components["schemas"]["Quantity"] | null;
            /** @description Speed and direction in the selected units, e.g. 8–16 km/h NW. */
            wind: string;
        };
        /** @description city-center when resolved from the city index; coordinates when supplied by the caller. */
        Precision: "city-center" | "coordinates";
        /** @description A measurement in the selected units. */
        Quantity: {
            unit: components["schemas"]["Unit"];
            /**
             * Format: double
             * @description Rounded to one decimal.
             */
            value: number;
        };
        /** @description An NWS query URL. */
        QuerySource: {
            url: string;
        };
        ReportSources: {
            alerts: components["schemas"]["QuerySource"];
            forecast: components["schemas"]["IssuedSource"];
            /** @description Absent when the grid lookup did not provide a safe hourly source URL. */
            hourly: components["schemas"]["IssuedSource"] | null;
        };
        /**
         * @description °F and mph for us units; °C and km/h for metric units.
         * @enum {string}
         */
        Unit: "°F" | "°C" | "mph" | "km/h";
        /**
         * @description Unit system for numeric temperatures and winds.
         * @enum {string}
         */
        Units: "us" | "metric";
        /** @description Full weather report: observation, forecast, hourly periods and alerts. */
        WeatherReport: {
            alerts: components["schemas"]["Alert"][];
            /** @description See AlertsStatus. unavailable means alerts could not be checked. */
            alertsStatus: components["schemas"]["CheckedAlertsStatus"];
            /**
             * Format: date-time
             * @description When this response was assembled, not when upstream data were observed.
             */
            assembledAt: string;
            /**
             * Format: uint64
             * @description Upstream responses are cached up to this many seconds.
             */
            cacheMaxAgeSeconds: number;
            /** @description Station observation or null; never a forecast. */
            current: components["schemas"]["Observation"] | null;
            forecast: components["schemas"]["Period"][];
            /** @description Empty when the hourly forecast failed (see warnings). */
            hourly: components["schemas"]["Period"][];
            location: components["schemas"]["Location"];
            sources: components["schemas"]["ReportSources"];
            /** @description Official forecast text for the first period. */
            summary: string;
            units: components["schemas"]["Units"];
            /** @description Human-readable notes. Source failures make the response partial (Cache-Control: no-store). */
            warnings: string[];
        };
        /** @description `{ "data": ..., "meta": ... }` wrapper for every successful response. */
        WeatherReportEnvelope: {
            data: components["schemas"]["WeatherReport"];
            meta: components["schemas"]["Meta"];
        };
    };
    responses: never;
    parameters: never;
    requestBodies: never;
    headers: never;
    pathItems: never;
}
export type $defs = Record<string, never>;
export interface operations {
    getMetrics: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description plain text */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "text/plain; charset=utf-8": unknown;
                };
            };
            /** @description Request did not come through the public address (FORBIDDEN). Only when the server is configured with an origin-verify value. */
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
        };
    };
    getActiveAlerts: {
        parameters: {
            query?: {
                /** @description Exact city name, optionally qualified with state, e.g. Seattle, WA. Ambiguous names return choices. */
                city?: string;
                /** @description GeoNames city ID from search_cities. Alternative to city or coordinates. */
                cityId?: number;
                /** @description Latitude. The NWS grid lookup uses the value rounded to two decimals (see location.gridLookupPoint); alerts use four decimals. */
                lat?: number;
                /** @description Longitude. The NWS grid lookup uses the value rounded to two decimals (see location.gridLookupPoint); alerts use four decimals. */
                lon?: number;
                /** @description Unit system for numeric temperatures and winds. */
                units?: "us" | "metric";
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Active alerts and alert-check status; an alert-check failure is alertsStatus unavailable with a warning. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ActiveAlertsEnvelope"];
                };
            };
            /** @description Invalid query, location or request body (INVALID_LOCATION) */
            400: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
            /** @description Request did not come through the public address (FORBIDDEN). Only when the server is configured with an origin-verify value. */
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
            /** @description City not found (CITY_NOT_FOUND); suggestions in choices when available */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
            /** @description Ambiguous city (AMBIGUOUS_CITY); select one of choices */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
            /** @description Request body exceeds 16 KiB (REQUEST_TOO_LARGE) */
            413: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
            /** @description Outside NWS coverage (OUTSIDE_COVERAGE) */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
            /** @description NWS unavailable (UPSTREAM_UNAVAILABLE) */
            502: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
            /** @description Too many in-flight requests (BUSY) */
            503: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
            /** @description Weather lookup or HTTP request deadline exceeded (UPSTREAM_TIMEOUT) */
            504: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
        };
    };
    searchCities: {
        parameters: {
            query: {
                q: string;
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Up to 10 exact or prefix matches ordered by population */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["CitiesEnvelope"];
                };
            };
            /** @description Invalid query, location or request body (INVALID_LOCATION) */
            400: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
            /** @description Request did not come through the public address (FORBIDDEN). Only when the server is configured with an origin-verify value. */
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
            /** @description Request body exceeds 16 KiB (REQUEST_TOO_LARGE) */
            413: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
            /** @description Weather lookup or HTTP request deadline exceeded (UPSTREAM_TIMEOUT) */
            504: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
        };
    };
    getHourlyForecast: {
        parameters: {
            query?: {
                /** @description Exact city name, optionally qualified with state, e.g. Seattle, WA. Ambiguous names return choices. */
                city?: string;
                /** @description GeoNames city ID from search_cities. Alternative to city or coordinates. */
                cityId?: number;
                /** @description Latitude. The NWS grid lookup uses the value rounded to two decimals (see location.gridLookupPoint); alerts use four decimals. */
                lat?: number;
                /** @description Longitude. The NWS grid lookup uses the value rounded to two decimals (see location.gridLookupPoint); alerts use four decimals. */
                lon?: number;
                /** @description Unit system for numeric temperatures and winds. */
                units?: "us" | "metric";
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Hourly forecast periods. A failed hourly fetch returns 502. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["HourlyForecastEnvelope"];
                };
            };
            /** @description Invalid query, location or request body (INVALID_LOCATION) */
            400: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
            /** @description Request did not come through the public address (FORBIDDEN). Only when the server is configured with an origin-verify value. */
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
            /** @description City not found (CITY_NOT_FOUND); suggestions in choices when available */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
            /** @description Ambiguous city (AMBIGUOUS_CITY); select one of choices */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
            /** @description Request body exceeds 16 KiB (REQUEST_TOO_LARGE) */
            413: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
            /** @description Outside NWS coverage (OUTSIDE_COVERAGE) */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
            /** @description NWS unavailable (UPSTREAM_UNAVAILABLE) */
            502: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
            /** @description Too many in-flight requests (BUSY) */
            503: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
            /** @description Weather lookup or HTTP request deadline exceeded (UPSTREAM_TIMEOUT) */
            504: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
        };
    };
    getWeather: {
        parameters: {
            query?: {
                /** @description Exact city name, optionally qualified with state, e.g. Seattle, WA. Ambiguous names return choices. */
                city?: string;
                /** @description GeoNames city ID from search_cities. Alternative to city or coordinates. */
                cityId?: number;
                /** @description Latitude. The NWS grid lookup uses the value rounded to two decimals (see location.gridLookupPoint); alerts use four decimals. */
                lat?: number;
                /** @description Longitude. The NWS grid lookup uses the value rounded to two decimals (see location.gridLookupPoint); alerts use four decimals. */
                lon?: number;
                /** @description Unit system for numeric temperatures and winds. */
                units?: "us" | "metric";
            };
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            /** @description Weather report; may contain partial-source warnings. */
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["WeatherReportEnvelope"];
                };
            };
            /** @description Invalid query, location or request body (INVALID_LOCATION) */
            400: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
            /** @description Request did not come through the public address (FORBIDDEN). Only when the server is configured with an origin-verify value. */
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
            /** @description City not found (CITY_NOT_FOUND); suggestions in choices when available */
            404: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
            /** @description Ambiguous city (AMBIGUOUS_CITY); select one of choices */
            409: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
            /** @description Request body exceeds 16 KiB (REQUEST_TOO_LARGE) */
            413: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
            /** @description Outside NWS coverage (OUTSIDE_COVERAGE) */
            422: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
            /** @description NWS unavailable (UPSTREAM_UNAVAILABLE) */
            502: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
            /** @description Too many in-flight requests (BUSY) */
            503: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
            /** @description Weather lookup or HTTP request deadline exceeded (UPSTREAM_TIMEOUT) */
            504: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
        };
    };
    getVersion: {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        requestBody?: never;
        responses: {
            200: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": {
                        revision: string;
                        version: string;
                    };
                };
            };
            /** @description Request did not come through the public address (FORBIDDEN). Only when the server is configured with an origin-verify value. */
            403: {
                headers: {
                    [name: string]: unknown;
                };
                content: {
                    "application/json": components["schemas"]["ErrorBody"];
                };
            };
        };
    };
}
