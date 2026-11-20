use crate::Ditado;
use chrono::{Datelike, Duration, Local, TimeZone, Timelike};
use std::collections::BTreeMap;
#[derive(Default, Debug, Clone, serde::Serialize)]
pub struct Dia {
    pub dia: String,
    pub palavras: i64,
    pub ditados: u64,
    pub minutos: f64,
}
#[derive(Default, Debug, Clone, serde::Serialize)]
pub struct App {
    pub app: String,
    pub nome: String,
    pub palavras: i64,
    pub ditados: u64,
}
#[derive(Default, Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Estatisticas {
    pub total_palavras: i64,
    pub total_ditados: u64,
    pub minutos_falados: f64,
    pub ppm_medio: f64,
    pub ppm_de_cada_ditado: Vec<f64>,
    pub minutos_economizados: f64,
    pub dias_seguidos: usize,
    pub maior_sequencia: usize,
    pub dias_ativos: usize,
    pub palavras_hoje: i64,
    pub palavras_na_semana: i64,
    pub palavras_na_semana_anterior: i64,
    pub maior_ditado_palavras: i64,
    pub maior_ditado_minutos: f64,
    pub latencia_mediana: f64,
    pub trocas_dicionario: i64,
    pub por_dia: Vec<Dia>,
    pub por_hora: [i64; 24],
    pub por_dia_da_semana: [i64; 7],
    pub por_app: Vec<App>,
}
impl Estatisticas {
    pub fn calcular(ditados: &[Ditado], agora: f64, dias_no_grafico: usize) -> Self {
        Self::calcular_no_fuso(ditados, agora, dias_no_grafico, Local)
    }
    pub fn calcular_no_fuso<T: TimeZone>(
        ditados: &[Ditado],
        agora: f64,
        dias_no_grafico: usize,
        fuso: T,
    ) -> Self {
        let mut e = Self::default();
        let Some(hoje) = fuso
            .timestamp_opt(agora.floor() as i64, 0)
            .single()
            .map(|d| d.date_naive())
        else {
            return e;
        };
        let mut dias = BTreeMap::new();
        let mut apps = BTreeMap::new();
        let mut com_tempo = 0;
        let mut latencias = vec![];
        for d in ditados.iter().filter(|d| d.palavras > 0) {
            let Some(data) = fuso.timestamp_opt(d.iniciado_em.floor() as i64, 0).single() else {
                continue;
            };
            let dia = data.date_naive();
            let fala = if d.duracao_fala > 0. {
                d.duracao_fala
            } else {
                d.duracao
            };
            e.total_palavras += d.palavras;
            e.total_ditados += 1;
            e.trocas_dicionario += d.trocas_dicionario;
            if fala >= 2. {
                e.minutos_falados += fala / 60.;
                com_tempo += d.palavras;
                let ppm = d.palavras as f64 * 60. / fala;
                if ppm > 20. && ppm < 400. {
                    e.ppm_de_cada_ditado.push(ppm.round())
                }
            }
            if d.origem == "ditado" && d.latencia > 0. {
                latencias.push(d.latencia)
            }
            if d.palavras > e.maior_ditado_palavras {
                e.maior_ditado_palavras = d.palavras;
                e.maior_ditado_minutos = d.duracao / 60.
            }
            let linha = dias.entry(dia).or_insert_with(|| Dia {
                dia: dia.to_string(),
                ..Default::default()
            });
            linha.palavras += d.palavras;
            linha.ditados += 1;
            linha.minutos += fala / 60.;
            e.por_hora[data.hour() as usize] += d.palavras;
            e.por_dia_da_semana[data.weekday().num_days_from_monday() as usize] += d.palavras;
            let id = if d.app.is_empty() {
                "desconhecido"
            } else {
                &d.app
            };
            let app = apps.entry(id).or_insert_with(|| App {
                app: id.into(),
                nome: id.into(),
                ..Default::default()
            });
            if !d.app_nome.is_empty() {
                app.nome = d.app_nome.clone()
            }
            app.palavras += d.palavras;
            app.ditados += 1;
            let dias_atras = (hoje - dia).num_days();
            if dias_atras <= 0 {
                e.palavras_hoje += d.palavras
            }
            if dias_atras <= 6 {
                e.palavras_na_semana += d.palavras
            } else if dias_atras <= 13 {
                e.palavras_na_semana_anterior += d.palavras
            }
        }
        if e.minutos_falados > 0. {
            e.ppm_medio = com_tempo as f64 / e.minutos_falados
        }
        e.minutos_economizados = (e.total_palavras as f64 / 40. - e.minutos_falados).max(0.);
        e.dias_ativos = dias.len();
        latencias.sort_by(f64::total_cmp);
        e.latencia_mediana = latencias
            .get(latencias.len() / 2)
            .copied()
            .unwrap_or_default();
        e.por_app = apps.into_values().collect();
        e.por_app.sort_by_key(|a| std::cmp::Reverse(a.palavras));
        for n in (0..dias_no_grafico).rev() {
            if let Some(d) = Duration::try_days(n as i64).and_then(|n| hoje.checked_sub_signed(n)) {
                e.por_dia.push(dias.get(&d).cloned().unwrap_or_else(|| Dia {
                    dia: d.to_string(),
                    ..Default::default()
                }))
            }
        }
        let mut cursor = if dias.contains_key(&hoje) {
            Some(hoje)
        } else {
            hoje.pred_opt()
        };
        while let Some(d) = cursor.filter(|d| dias.contains_key(d)) {
            e.dias_seguidos += 1;
            cursor = d.pred_opt()
        }
        let mut ultimo = None;
        let mut corrida = 0;
        for d in dias.keys() {
            corrida = if ultimo.is_some() && d.pred_opt() == ultimo {
                corrida + 1
            } else {
                1
            };
            e.maior_sequencia = e.maior_sequencia.max(corrida);
            ultimo = Some(*d)
        }
        e
    }
}
