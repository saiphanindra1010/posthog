import { TZLabel } from '@posthog/apps-common'
import { IconCheck } from '@posthog/icons'
import { Tooltip } from '@posthog/lemon-ui'
import { useActions, useValues } from 'kea'
import { router } from 'kea-router'
import { LemonTable, LemonTableColumn, LemonTableColumns } from 'lib/lemon-ui/LemonTable'
import { createdByColumn } from 'lib/lemon-ui/LemonTable/columnUtils'
import { LemonTableLink } from 'lib/lemon-ui/LemonTable/LemonTableLink'
import { urls } from 'scenes/urls'

import { AlertState } from '../../../../queries/schema'
import { alertLogic } from '../alertLogic'
import { alertsLogic } from '../alertsLogic'
import { AlertType } from '../types'
import { EditAlertModal } from './EditAlertModal'
import { AlertStateIndicator } from './ManageAlertsModal'

interface AlertsProps {
    alertId: AlertType['id'] | null
}

export function Alerts({ alertId }: AlertsProps): JSX.Element {
    const { push } = useActions(router)
    const logic = alertsLogic()
    const { loadAlerts } = useActions(logic)

    const { alert } = useValues(alertLogic({ alertId }))

    const { alertsSortedByState, alertsLoading } = useValues(logic)

    const columns: LemonTableColumns<AlertType> = [
        {
            key: 'id',
            width: 32,
        },
        {
            title: 'Name',
            dataIndex: 'name',
            key: 'name',
            render: function renderName(name: any, alert) {
                return (
                    <>
                        <LemonTableLink
                            to={urls.alert(alert.id)}
                            className={alert.enabled ? '' : 'text-muted'}
                            title={
                                <div className="flex flex-row gap-3 items-center">
                                    {alert.enabled ? <AlertStateIndicator alert={alert} /> : null}
                                    <div>{name}</div>
                                </div>
                            }
                        />
                    </>
                )
            },
        },
        {
            title: 'Last checked',
            sorter: true,
            dataIndex: 'last_checked_at',
            render: function renderLastChecked(last_checked_at: any) {
                return <div className="whitespace-nowrap">{last_checked_at && <TZLabel time={last_checked_at} />}</div>
            },
        },
        {
            title: 'Last notified',
            sorter: true,
            dataIndex: 'last_notified_at',
            render: function renderLastModified(last_notified_at: any) {
                return (
                    <div className="whitespace-nowrap">{last_notified_at && <TZLabel time={last_notified_at} />}</div>
                )
            },
        },
        createdByColumn() as LemonTableColumn<AlertType, keyof AlertType | undefined>,
        {
            title: 'Insight',
            dataIndex: 'insight',
            key: 'insight',
            render: function renderInsightLink(insight: any) {
                return (
                    <LemonTableLink
                        to={urls.insightView(insight.short_id)}
                        title={
                            <Tooltip title={insight.name}>
                                <div>{insight.name ?? insight.derived_name}</div>
                            </Tooltip>
                        }
                    />
                )
            },
        },
        {
            title: 'Enabled',
            dataIndex: 'enabled',
            key: 'enabled',
            render: (enabled: any) => (enabled ? <IconCheck /> : null),
        },
    ]

    // TODO: add info here to sign up for alerts early access
    return (
        <>
            {alert && (
                <EditAlertModal
                    onClose={() => push(urls.alerts())}
                    isOpen
                    alertId={alert.id}
                    insightShortId={alert.insight.short_id}
                    insightId={alert.insight.id}
                    onEditSuccess={() => {
                        loadAlerts()
                        push(urls.alerts())
                    }}
                />
            )}

            <LemonTable
                loading={alertsLoading}
                columns={columns}
                dataSource={alertsSortedByState}
                noSortingCancellation
                rowKey="id"
                loadingSkeletonRows={5}
                nouns={['alert', 'alerts']}
                rowClassName={(alert) => (alert.state === AlertState.NOT_FIRING ? null : 'highlighted')}
            />
        </>
    )
}