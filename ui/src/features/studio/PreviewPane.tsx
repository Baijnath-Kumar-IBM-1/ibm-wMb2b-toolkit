import { Button, CodeSnippet, InlineLoading, Tab, TabList, TabPanel, TabPanels, Tabs, Tag } from '@carbon/react'
import { Copy, Document, Download } from '@carbon/icons-react'
import type { PreviewReport } from '../../api/types'
import { ErrorNotice } from '../../components/States'
import { controlNumber, downloadText, segmentCount } from '../../lib/format'

export function PreviewPane({
  report,
  error,
  loading,
  onRetry,
  idle,
}: {
  report: PreviewReport | null
  error: Error | null
  loading: boolean
  onRetry: () => void
  idle: string
}) {
  if (error) return <ErrorNotice title="Preview failed" error={error} onRetry={onRetry} />
  if (!report) {
    return (
      <div className="preview-empty">
        {loading ? <InlineLoading description="Rendering" /> : <Document size={32} />}
        <h3>{loading ? 'Rendering preview' : 'Nothing to preview yet'}</h3>
        <p>{idle}</p>
      </div>
    )
  }
  return (
    <Tabs>
      <TabList aria-label="Selected document previews" contained>
        {report.documents.map((d) => (
          <Tab key={d.transaction_code}>{d.transaction_code}</Tab>
        ))}
      </TabList>
      <TabPanels>
        {report.documents.map((d) => (
          <TabPanel key={d.transaction_code}>
            <div className="preview-heading">
              <div>
                <Tag type="cyan">EDI</Tag>
                <div>
                  <h3>{d.transaction_code}.edi</h3>
                  <p>
                    {d.name} · {segmentCount(d.payload)} segments · ISA {controlNumber(d.isa_control)} · GS{' '}
                    {d.gs_control}
                  </p>
                </div>
              </div>
              <div className="preview-actions">
                <Button
                  kind="ghost"
                  size="sm"
                  renderIcon={Copy}
                  onClick={() => void navigator.clipboard?.writeText(d.payload)}
                >
                  Copy
                </Button>
                <Button
                  kind="ghost"
                  size="sm"
                  renderIcon={Download}
                  onClick={() => downloadText(`${d.transaction_code}.edi`, d.payload)}
                >
                  Download
                </Button>
              </div>
            </div>
            <CodeSnippet type="multi" feedback="Copied to clipboard" wrapText>
              {d.payload.split('~').join('~\n')}
            </CodeSnippet>
          </TabPanel>
        ))}
      </TabPanels>
    </Tabs>
  )
}
