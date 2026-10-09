import { createWeatherClient, LatestRequest } from './api.ts';
import type { components, paths } from './schema.d.ts';
type City = components['schemas']['City'];
type WeatherQuery = NonNullable<paths['/v1/weather']['get']['parameters']['query']>;

type PageEnvironment = Pick<
    typeof globalThis,
    | 'document'
    | 'location'
    | 'HTMLInputElement'
    | 'HTMLSelectElement'
    | 'HTMLButtonElement'
    | 'HTMLAnchorElement'
    | 'fetch'
    | 'Date'
    | 'setTimeout'
    | 'clearTimeout'
>;

export function mountWeatherPage(environment: PageEnvironment): void {
    const {
        document,
        location,
        HTMLInputElement,
        HTMLSelectElement,
        HTMLButtonElement,
        HTMLAnchorElement,
        fetch,
        Date,
        setTimeout,
        clearTimeout,
    } = environment;
    const api = createWeatherClient(location.origin, fetch);

    const searchRequests = new LatestRequest();
    function $(id: string): HTMLElement {
        const node = document.getElementById(id);
        if (!node) throw new Error(`Missing page element: ${id}`);
        return node;
    }
    function input(id: string) {
        const node = $(id);
        if (!(node instanceof HTMLInputElement)) throw new Error(`Invalid input: ${id}`);
        return node;
    }
    function select(id: string) {
        const node = $(id);
        if (!(node instanceof HTMLSelectElement)) throw new Error(`Invalid select: ${id}`);
        return node;
    }
    function button(id: string) {
        const node = $(id);
        if (!(node instanceof HTMLButtonElement)) throw new Error(`Invalid button: ${id}`);
        return node;
    }
    function link(id: string) {
        const node = $(id);
        if (!(node instanceof HTMLAnchorElement)) throw new Error(`Invalid link: ${id}`);
        return node;
    }
    const cityInput = input('city');
    const unitsInput = select('units');
    const go = button('go');
    const jsonLink = link('jsonLink');
    let selected: City | null = null;
    let lastQuery: WeatherQuery = { city: cityInput.value };
    let timer: ReturnType<typeof setTimeout> | undefined;
    let reportZone = 'UTC';
    const text = (id: string, value: string) => {
        $(id).textContent = value;
    };
    function el<K extends keyof HTMLElementTagNameMap>(
        tag: K,
        className: string | null,
        value?: string | null,
    ) {
        const node = document.createElement(tag);
        if (className) node.className = className;
        if (value !== undefined) node.textContent = value;
        return node;
    }
    function units(): components['schemas']['Units'] {
        return unitsInput.value === 'metric' ? 'metric' : 'us';
    }
    function time(value: string | null, options: Intl.DateTimeFormatOptions = {}) {
        if (!value) return 'Unavailable';
        const date = new Date(value);
        if (Number.isNaN(date.getTime())) return 'Unavailable';
        return new Intl.DateTimeFormat('en-US', { timeZone: reportZone, ...options }).format(date);
    }
    function quantity(q: components['schemas']['Quantity'] | null) {
        return q && typeof q.value === 'number'
            ? `${Math.round(q.value)} ${q.unit}`
            : 'Unavailable';
    }
    function developer(query: WeatherQuery) {
        const params = new URLSearchParams();
        for (const [key, value] of Object.entries({ ...query, units: units() })) {
            if (value !== undefined) params.set(key, String(value));
        }
        const path = '/v1/weather?' + params;
        text('curl', `curl '${location.origin}${path}'`);
        jsonLink.href = path;
        text('mcpUrl', location.origin + '/mcp');
    }
    function choices(cities: City[]) {
        $('suggestions').replaceChildren();
        for (const city of cities) {
            const button = el('button', null, `${city.name}, ${city.state}`);
            button.type = 'button';
            button.append(el('span', null, `${city.stateName} · ${city.country}`));
            button.addEventListener('click', () => {
                selected = city;
                cityInput.value = `${city.name}, ${city.state}`;
                cancelSearch();
                closeChoices();
                void load({ cityId: city.id });
            });
            $('suggestions').append(button);
        }
        $('suggestions').hidden = !cities.length;
        cityInput.setAttribute('aria-expanded', String(Boolean(cities.length)));
    }
    function closeChoices() {
        $('suggestions').hidden = true;
        cityInput.setAttribute('aria-expanded', 'false');
    }
    function cancelSearch() {
        clearTimeout(timer);
        searchRequests.cancel();
    }
    cityInput.addEventListener('input', () => {
        selected = null;
        cancelSearch();
        timer = setTimeout(async () => {
            if (cityInput.value.trim().length < 2) {
                closeChoices();
                return;
            }
            const controller = searchRequests.start();
            try {
                const { data } = await api.GET('/v1/cities', {
                    params: { query: { q: cityInput.value } },
                    signal: controller.signal,
                });
                if (searchRequests.isCurrent(controller)) choices(data?.data || []);
            } catch {
                if (searchRequests.isCurrent(controller)) closeChoices();
            }
        }, 200);
    });
    cityInput.addEventListener('keydown', (event) => {
        if (event.key === 'Escape') closeChoices();
        if (event.key === 'ArrowDown' && !$('suggestions').hidden) {
            event.preventDefault();
            $('suggestions').querySelector('button')?.focus();
        }
    });
    $('suggestions').addEventListener('keydown', (event) => {
        const buttons = [...$('suggestions').querySelectorAll('button')];
        const index = buttons.findIndex((button) => button === document.activeElement);
        if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
            event.preventDefault();
            buttons[
                (index + (event.key === 'ArrowDown' ? 1 : -1) + buttons.length) % buttons.length
            ]?.focus();
        }
        if (event.key === 'Escape') {
            closeChoices();
            cityInput.focus();
        }
    });
    $('search').addEventListener('submit', (event) => {
        event.preventDefault();
        cancelSearch();
        closeChoices();
        void load(selected ? { cityId: selected.id } : { city: cityInput.value.trim() });
    });
    unitsInput.addEventListener('change', () => {
        void load(lastQuery);
    });
    // Match the service's two-minute source window; hidden tabs resume on visibility.
    const REFRESH_MS = 120000,
        REQUEST_TIMEOUT_MS = 55000;
    let refreshTimer: ReturnType<typeof setTimeout> | undefined;
    let expiryTimer: ReturnType<typeof setTimeout> | undefined;
    let displayedReport: components['schemas']['WeatherReport'] | null = null;
    let weatherController: AbortController | null = null;
    let freshUntil = 0,
        reportExpired = false;
    function freshness() {
        if (!displayedReport) return;
        const expired = reportExpired || Date.now() >= freshUntil;
        text(
            'freshness',
            `Report assembled ${time(displayedReport.assembledAt, { hour: 'numeric', minute: '2-digit', second: '2-digit' })} · ${expired ? 'Weather needs refresh' : 'Refreshes automatically while this tab is visible'}`,
        );
        const badge = $('alertBadge'),
            d = displayedReport;
        badge.className =
            'tag' + (expired || d.alerts.length || d.alertsStatus !== 'checked' ? ' warning' : '');
        badge.textContent = expired
            ? d.alerts.length
                ? `Previous alerts · ${d.alerts.length}`
                : 'Alerts need refresh'
            : d.alertsStatus !== 'checked'
              ? 'Alerts unavailable'
              : d.alerts.length
                ? `${d.alerts.length} active alert${d.alerts.length === 1 ? '' : 's'}`
                : 'No active alerts';
        const c = d.current;
        if (c) {
            const observed = Date.parse(c.observedAt);
            const stale = c.stale || !Number.isFinite(observed) || Date.now() - observed > 7200000;
            text(
                'observed',
                `${stale ? 'Older observation · ' : ''}${time(c.observedAt, { weekday: 'short', hour: 'numeric', minute: '2-digit' })} · Station ${c.station}, ${c.stationDistanceKm} km away`,
            );
        }
    }
    function scheduleRefresh(delay = REFRESH_MS) {
        clearTimeout(refreshTimer);
        clearTimeout(expiryTimer);
        if (displayedReport && !reportExpired && freshUntil > Date.now()) {
            expiryTimer = setTimeout(freshness, freshUntil - Date.now());
        }
        refreshTimer = setTimeout(() => {
            freshness();
            if (!document.hidden && !weatherController) void load(lastQuery, { refresh: true });
        }, delay);
    }
    document.addEventListener('visibilitychange', () => {
        freshness();
        if (
            !document.hidden &&
            displayedReport &&
            !weatherController &&
            (reportExpired || Date.now() >= freshUntil)
        )
            void load(lastQuery, { refresh: true });
    });
    async function load(query: WeatherQuery, { refresh = false } = {}) {
        clearTimeout(refreshTimer);
        weatherController?.abort();
        const controller = new AbortController();
        weatherController = controller;
        let timedOut = false;
        let failureMessage = 'Weather is temporarily unavailable. Try again.';
        const requestTimer = setTimeout(() => {
            timedOut = true;
            controller.abort();
        }, REQUEST_TIMEOUT_MS);
        lastQuery = query;
        developer(query);
        go.disabled = true;
        text(
            'status',
            refresh
                ? 'Refreshing weather and official alerts…'
                : 'Checking the forecast, nearby stations, and official alerts…',
        );
        $('status').className = 'status';
        if (!refresh) {
            clearTimeout(expiryTimer);
            displayedReport = null;
            $('weather').hidden = true;
        }
        $('empty').hidden = true;
        freshness();
        try {
            const {
                data,
                error,
                response: res,
            } = await api.GET('/v1/weather', {
                params: { query: { ...query, units: units() } },
                signal: controller.signal,
            });
            if (controller !== weatherController) return;
            if (!res.ok || !data) {
                const first = error?.errors?.[0];
                if (!refresh) choices(first?.choices || []);
                failureMessage = first?.detail || failureMessage;
                throw new Error(failureMessage);
            }
            const body = data;
            render(body.data);
            displayedReport = body.data;
            reportExpired = false;
            const cacheControl = res.headers.get('Cache-Control') || '';
            const maxAge = cacheControl.match(/(?:^|[,\s])max-age=(\d+)/);
            const age = Math.max(0, Number(res.headers.get('Age')) || 0);
            const remaining = maxAge ? Math.max(0, Number(maxAge[1]) - age) * 1000 : REFRESH_MS;
            const assembled = Date.parse(body.data.assembledAt);
            freshUntil = Math.min(
                Date.now() + remaining,
                Number.isFinite(assembled) ? assembled + REFRESH_MS : Date.now(),
            );
            freshness();
            text('status', '');
            $('weather').hidden = false;
            // A near-expired CDN response must not create a tight polling loop.
            scheduleRefresh(Math.max(10000, Math.min(REFRESH_MS, freshUntil - Date.now())));
        } catch (e) {
            if (controller !== weatherController) return;
            if (!(e instanceof Error) || e.name !== 'AbortError' || timedOut) {
                if (refresh && displayedReport) {
                    reportExpired = true;
                    freshness();
                    scheduleRefresh();
                }
                text(
                    'status',
                    `${timedOut ? 'Weather refresh timed out. Please retry.' : failureMessage}${refresh ? ' Displayed weather may be out of date; retrying automatically.' : ''}`,
                );
                $('status').className = 'status error';
            }
        } finally {
            clearTimeout(requestTimer);
            if (controller === weatherController) {
                weatherController = null;
                go.disabled = false;
            }
        }
    }
    function render(d: components['schemas']['WeatherReport']) {
        reportZone = d.location.timeZone || 'UTC';
        text('location', d.location.name);
        text(
            'locationNote',
            `${d.location.precision === 'city-center' ? 'City center' : 'Selected coordinates'} · ${d.location.latitude.toFixed(3)}, ${d.location.longitude.toFixed(3)} · ${reportZone}`,
        );
        const current = d.current;
        if (current) {
            $('temperature').replaceChildren(
                el(
                    'span',
                    null,
                    current.temperature ? String(Math.round(current.temperature.value)) : '—',
                ),
                el('small', null, current.temperature?.unit || ''),
            );
            text('condition', current.condition || 'Conditions unavailable');
            text('wind', quantity(current.windSpeed));
            text(
                'humidity',
                typeof current.humidityPercent === 'number'
                    ? `${Math.round(current.humidityPercent)}%`
                    : 'Unavailable',
            );
        } else {
            text('temperature', '—');
            text('condition', 'Observation unavailable');
            text('wind', '—');
            text('humidity', '—');
            text('observed', 'See the forecast alongside.');
        }
        const firstPeriod = d.forecast[0];
        text(
            'outlookTitle',
            firstPeriod ? `${firstPeriod.name}: ${firstPeriod.condition}` : 'Forecast',
        );
        text('summary', d.summary);
        $('proseUnits').hidden = d.units !== 'metric';
        text(
            'issued',
            `Forecast issued ${time(d.sources.forecast.issuedAt, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' })}`,
        );
        $('alerts').replaceChildren();
        for (const alert of d.alerts) {
            const details = el('details', 'alert');
            details.append(
                el('summary', null, alert.headline || alert.event),
                el('pre', null, alert.description || ''),
                el('pre', null, alert.instruction || ''),
            );
            $('alerts').append(details);
        }
        $('warnings').replaceChildren(...d.warnings.map((warning) => el('p', null, warning)));
        renderHours(d.hourly);
        $('days').replaceChildren();
        for (const period of d.forecast.filter((period) => period.isDaytime).slice(0, 4)) {
            const card = el('article', 'day');
            card.append(
                el('h4', null, period.name || 'Forecast'),
                el('div', 'value', quantity(period.temperature)),
                el('p', null, period.condition),
                el('p', null, `Wind ${period.wind}`),
            );
            $('days').append(card);
        }
    }
    function renderHours(periods: components['schemas']['Period'][]) {
        $('chart').replaceChildren();
        $('hours').replaceChildren();
        if (!periods.length) {
            text('chart', 'Hourly forecast is unavailable.');
            return;
        }
        const selected = periods.filter((_, index) => index % 3 === 0).slice(0, 8);
        for (const period of selected) {
            const cell = el('div', 'hour', time(period.startsAt, { hour: 'numeric' }));
            cell.append(
                el(
                    'strong',
                    null,
                    period.temperature ? `${Math.round(period.temperature.value)}°` : '—',
                ),
                el(
                    'div',
                    'rain',
                    period.precipitationProbabilityPercent === null
                        ? 'Rain chance unknown'
                        : `${period.precipitationProbabilityPercent}% rain`,
                ),
            );
            cell.title = period.condition || 'Conditions unavailable';
            $('hours').append(cell);
        }
        const values = selected.map((period) => period.temperature?.value);
        if (!values.every((value): value is number => typeof value === 'number')) return;
        const ns = 'http://www.w3.org/2000/svg';
        const svg = document.createElementNS(ns, 'svg');
        svg.setAttribute('viewBox', '0 0 800 130');
        svg.setAttribute('role', 'img');
        svg.setAttribute('aria-label', 'Hourly temperature trend');
        const low = Math.min(...values) - 3;
        const high = Math.max(...values) + 3;
        const points = values.map((value, index) => [
            50 + index * 100,
            105 - ((value - low) / (high - low)) * 80,
        ]);
        const area = document.createElementNS(ns, 'path');
        area.setAttribute(
            'd',
            `M50 130 L${points.map((point) => point.join(' ')).join(' L')} L${50 + (values.length - 1) * 100} 130 Z`,
        );
        area.setAttribute('fill', '#e5f2f7');
        svg.append(area);
        const line = document.createElementNS(ns, 'polyline');
        line.setAttribute('points', points.map((point) => point.join(',')).join(' '));
        line.setAttribute('fill', 'none');
        line.setAttribute('stroke', '#076a93');
        line.setAttribute('stroke-width', '2.5');
        svg.append(line);
        for (const [x, y] of points) {
            const dot = document.createElementNS(ns, 'circle');
            dot.setAttribute('cx', String(x));
            dot.setAttribute('cy', String(y));
            dot.setAttribute('r', '4');
            dot.setAttribute('fill', '#076a93');
            svg.append(dot);
        }
        $('chart').append(svg);
    }
    developer(lastQuery);
}
